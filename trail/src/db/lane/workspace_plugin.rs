use std::ffi::{OsStr, OsString};
use std::io::{Cursor, Read, Write};
use std::process::{Child, Command, Stdio};
use std::thread;

use ed25519_dalek::{Signature, VerifyingKey};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use serde::{Deserialize, Serialize};
use trail_environment_adapter_sdk::{
    read_frame, write_frame, AdapterAction, AdapterCache, AdapterCacheAccess, AdapterCacheProtocol,
    AdapterCommand, AdapterDependencyType, AdapterExternalArtifact, AdapterHost, AdapterOperation,
    AdapterOutput, AdapterOutputPolicy, AdapterPackageManifest, AdapterPackageSignature,
    AdapterPermissions, AdapterPlan, AdapterPlanV2, AdapterPortability, AdapterPublisherKey,
    AdapterRequest, AdapterResponse, AdapterResult, AdapterRuntimeResource, DiscoveredComponent,
    PinnedFile, MAX_FRAME_BYTES, PACKAGE_SCHEMA_V1, PACKAGE_SIGNATURE_SCHEMA_V1, PROTOCOL_V1,
    PROTOCOL_V2, TRUSTED_PUBLISHER_KEY_SCHEMA_V1,
};

use super::workspace_environment::{
    workspace_environment_temporary_parent, WorkspaceEnvironmentCache,
    WorkspaceEnvironmentCacheAccess, WorkspaceEnvironmentCacheProtocol,
    WorkspaceEnvironmentCommand, WorkspaceEnvironmentDependency, WorkspaceEnvironmentEdgeType,
    WorkspaceEnvironmentExternalArtifact, WorkspaceEnvironmentInput, WorkspaceEnvironmentOutput,
    WorkspaceEnvironmentOutputPolicy, WorkspaceEnvironmentPlan,
    WorkspaceEnvironmentRuntimeResource, WorkspaceEnvironmentSandboxPolicy,
    WorkspaceEnvironmentSecretReference,
};
use super::*;

const PLUGIN_STORE_VERSION: &str = "v1";
const PLUGIN_PACKAGE_MANIFEST: &str = "trail-adapter.toml";
const PLUGIN_PACKAGE_SIGNATURE: &str = "trail-adapter.sig";
const INSTALLED_MANIFEST_FILE: &str = "manifest.cbor";
const REGISTRY_RECORD_SCHEMA: &str = "trail.environment-adapter-registry/v1";
const TRUST_RECORD_SCHEMA: &str = "trail.environment-adapter-publisher-trust/v1";
const MAX_PACKAGE_MANIFEST_BYTES: u64 = 1024 * 1024;
const MAX_PUBLISHER_DOCUMENT_BYTES: u64 = 64 * 1024;
const MAX_PLUGIN_EXECUTABLE_BYTES: u64 = 256 * 1024 * 1024;
const HARD_MAX_INPUT_FILES: u32 = 100_000;
const HARD_MAX_INPUT_BYTES: u64 = 8 * 1024 * 1024;
const HARD_MAX_TIMEOUT_MS: u64 = 30_000;
const HARD_MAX_RESPONSE_BYTES: u64 = 16 * 1024 * 1024;
const PLUGIN_MEMORY_LIMIT_BYTES: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InstalledPluginManifest {
    package: AdapterPackageManifest,
    #[serde(default)]
    payload_digest: String,
    distribution_digest: String,
    executable_digest: String,
    executable_file: String,
    #[serde(default)]
    signature: Option<AdapterPackageSignature>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginRegistryRecord {
    schema: String,
    canonical_identity: String,
    action: String,
    distribution_digest: Option<String>,
    recorded_at: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublisherTrustRecord {
    schema: String,
    key_id: String,
    publisher: String,
    public_key: String,
    action: String,
    recorded_at: i64,
}

struct SelectedPluginFiles {
    protocol_files: Vec<PinnedFile>,
    entries: BTreeMap<String, (String, FileEntry)>,
}

struct ProposedPluginPlan {
    component_id: String,
    kind: String,
    dependencies: Vec<WorkspaceEnvironmentDependency>,
    identity_inputs: Vec<String>,
    semantic_inputs: BTreeMap<String, String>,
    caches: Vec<AdapterCache>,
    external_artifacts: Vec<AdapterExternalArtifact>,
    runtime_resources: Vec<AdapterRuntimeResource>,
    command: Option<AdapterCommand>,
    mounted_commands: Vec<AdapterCommand>,
    outputs: Vec<AdapterOutput>,
    portability: AdapterPortability,
    stale_reason: String,
}

impl From<AdapterPlan> for ProposedPluginPlan {
    fn from(plan: AdapterPlan) -> Self {
        Self {
            component_id: plan.component_id,
            kind: plan.kind,
            dependencies: plan
                .dependencies
                .into_iter()
                .map(WorkspaceEnvironmentDependency::build_requires)
                .collect(),
            identity_inputs: plan.identity_inputs,
            semantic_inputs: plan.semantic_inputs,
            caches: Vec::new(),
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            command: Some(plan.command),
            mounted_commands: Vec::new(),
            outputs: plan.outputs,
            portability: plan.portability,
            stale_reason: plan.stale_reason,
        }
    }
}

impl ProposedPluginPlan {
    fn from_v2(plan: AdapterPlanV2) -> Result<Self> {
        let metadata_only = !plan.external_artifacts.is_empty();
        if (!metadata_only && plan.actions.is_empty()) || plan.actions.len() > 9 {
            return Err(Error::InvalidInput(
                "adapter protocol-v2 filesystem plan must declare between one and nine actions"
                    .to_string(),
            ));
        }
        if metadata_only
            && (plan.kind != "external"
                || !plan.actions.is_empty()
                || !plan.caches.is_empty()
                || !plan.outputs.is_empty())
        {
            return Err(Error::InvalidInput(
                "adapter protocol-v2 external-artifact plan must use kind `external` and cannot mix actions, caches, or filesystem outputs"
                    .to_string(),
            ));
        }
        let mut command = None;
        let mut mounted_commands = Vec::new();
        for action in plan.actions {
            match action {
                AdapterAction::Staging(candidate) => {
                    if command.replace(candidate).is_some() {
                        return Err(Error::InvalidInput(
                            "adapter protocol-v2 plan may declare at most one staging action"
                                .to_string(),
                        ));
                    }
                }
                AdapterAction::MountedInitialization(candidate) => mounted_commands.push(candidate),
            }
        }
        if mounted_commands.len() > 8 {
            return Err(Error::InvalidInput(
                "adapter protocol-v2 plan may declare at most eight mounted actions".to_string(),
            ));
        }
        let dependencies = plan
            .dependencies
            .into_iter()
            .map(WorkspaceEnvironmentDependency::build_requires)
            .chain(plan.dependency_edges.into_iter().map(|dependency| {
                WorkspaceEnvironmentDependency {
                    component_id: dependency.component_id,
                    edge_type: match dependency.edge_type {
                        AdapterDependencyType::BuildRequires => {
                            WorkspaceEnvironmentEdgeType::BuildRequires
                        }
                        AdapterDependencyType::RuntimeRequires => {
                            WorkspaceEnvironmentEdgeType::RuntimeRequires
                        }
                        AdapterDependencyType::BindsAfter => {
                            WorkspaceEnvironmentEdgeType::BindsAfter
                        }
                        AdapterDependencyType::InvalidatesWith => {
                            WorkspaceEnvironmentEdgeType::InvalidatesWith
                        }
                    },
                }
            }))
            .collect();
        Ok(Self {
            component_id: plan.component_id,
            kind: plan.kind,
            dependencies,
            identity_inputs: plan.identity_inputs,
            semantic_inputs: plan.semantic_inputs,
            caches: plan.caches,
            external_artifacts: plan.external_artifacts,
            runtime_resources: plan.runtime_resources,
            command,
            mounted_commands,
            outputs: plan.outputs,
            portability: plan.portability,
            stale_reason: plan.stale_reason,
        })
    }
}

struct BoundedRead {
    bytes: Vec<u8>,
    overflow: bool,
}

struct PreparedEnvironmentPluginPackage {
    package: AdapterPackageManifest,
    executable_bytes: Vec<u8>,
    executable_permissions: fs::Permissions,
    executable_digest: String,
    payload_material: Vec<u8>,
    payload_digest: String,
    signature: Option<AdapterPackageSignature>,
}

#[derive(Clone, Debug)]
pub(super) struct InstalledEnvironmentPlugin {
    pub(super) manifest: AdapterPackageManifest,
    pub(super) distribution_digest: String,
    pub(super) executable_digest: String,
    pub(super) executable_path: PathBuf,
    pub(super) publisher: Option<String>,
    pub(super) publisher_key_id: Option<String>,
    pub(super) trust: String,
    pub(super) certification_tier: String,
}

impl Trail {
    pub fn trust_environment_adapter_publisher_key(
        &self,
        key_document: impl AsRef<Path>,
    ) -> Result<EnvironmentPublisherTrustMutationReport> {
        let key_document = fs::canonicalize(key_document)?;
        let metadata = fs::symlink_metadata(&key_document)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_PUBLISHER_DOCUMENT_BYTES
        {
            return Err(Error::InvalidInput(format!(
                "publisher key document `{}` must be a bounded regular file",
                key_document.display()
            )));
        }
        let document: AdapterPublisherKey = toml::from_str(&fs::read_to_string(&key_document)?)
            .map_err(|error| {
                Error::InvalidInput(format!(
                    "cannot parse publisher key document `{}`: {error}",
                    key_document.display()
                ))
            })?;
        let (publisher, public_key, key_id) = validate_publisher_key_document(&document)?;
        self.append_publisher_trust_record(PublisherTrustRecord {
            schema: TRUST_RECORD_SCHEMA.to_string(),
            key_id: key_id.clone(),
            publisher: publisher.clone(),
            public_key,
            action: "trust".to_string(),
            recorded_at: now_ts(),
        })?;
        Ok(EnvironmentPublisherTrustMutationReport {
            publisher: Some(publisher),
            key_id,
            action: "trust".to_string(),
        })
    }

    pub fn remove_environment_adapter_publisher_key(
        &self,
        key_id: &str,
    ) -> Result<EnvironmentPublisherTrustMutationReport> {
        validate_publisher_key_id(key_id)?;
        let current = self.latest_publisher_trust_record(key_id)?;
        let publisher = current
            .as_ref()
            .filter(|record| record.action == "trust")
            .map(|record| record.publisher.clone());
        let public_key = current
            .as_ref()
            .map(|record| record.public_key.clone())
            .unwrap_or_default();
        self.append_publisher_trust_record(PublisherTrustRecord {
            schema: TRUST_RECORD_SCHEMA.to_string(),
            key_id: key_id.to_string(),
            publisher: publisher.clone().unwrap_or_default(),
            public_key,
            action: "remove".to_string(),
            recorded_at: now_ts(),
        })?;
        Ok(EnvironmentPublisherTrustMutationReport {
            publisher,
            key_id: key_id.to_string(),
            action: "remove".to_string(),
        })
    }

    pub fn environment_adapter_publisher_trust(&self) -> Result<EnvironmentPublisherTrustReport> {
        let root = self.environment_plugin_store_root().join("trust");
        if !root.exists() {
            return Ok(EnvironmentPublisherTrustReport { keys: Vec::new() });
        }
        let mut keys = Vec::new();
        for directory in fs::read_dir(&root)? {
            let directory = directory?;
            if !directory.file_type()?.is_dir() {
                return Err(Error::Corrupt(format!(
                    "publisher trust store contains non-directory `{}`",
                    directory.path().display()
                )));
            }
            let mut records =
                fs::read_dir(directory.path())?.collect::<std::result::Result<Vec<_>, _>>()?;
            records.retain(|entry| {
                entry.file_type().is_ok_and(|kind| kind.is_file())
                    && entry.path().extension().and_then(|value| value.to_str()) == Some("cbor")
            });
            records.sort_by_key(|entry| entry.file_name());
            let Some(latest) = records.last() else {
                continue;
            };
            let record: PublisherTrustRecord = serde_cbor::from_slice(&fs::read(latest.path())?)
                .map_err(|error| {
                    Error::Corrupt(format!(
                        "cannot decode publisher trust record `{}`: {error}",
                        latest.path().display()
                    ))
                })?;
            validate_publisher_trust_record(&record, &directory.file_name())?;
            if record.action == "trust" {
                keys.push(EnvironmentPublisherTrustEntryReport {
                    publisher: record.publisher,
                    key_id: record.key_id,
                    public_key: record.public_key,
                    trusted_at: record.recorded_at,
                });
            }
        }
        keys.sort_by(|left, right| left.key_id.cmp(&right.key_id));
        Ok(EnvironmentPublisherTrustReport { keys })
    }

    pub fn inspect_environment_adapter_plugin_package(
        &self,
        package_directory: impl AsRef<Path>,
    ) -> Result<EnvironmentPluginPackageInspectionReport> {
        let prepared = prepare_environment_plugin_package(package_directory)?;
        let distribution_material = adapter_package_distribution_material(
            &prepared.payload_material,
            prepared.signature.as_ref(),
        )?;
        Ok(EnvironmentPluginPackageInspectionReport {
            canonical_identity: prepared.package.adapter.canonical_identity,
            payload_digest: prepared.payload_digest,
            executable_digest: prepared.executable_digest,
            distribution_digest: format!("sha256:{}", sha256_hex(&distribution_material)),
            signature_present: prepared.signature.is_some(),
            publisher: prepared
                .signature
                .as_ref()
                .map(|signature| signature.publisher.clone()),
            publisher_key_id: prepared.signature.map(|signature| signature.key_id),
        })
    }

    pub fn install_environment_adapter_plugin(
        &self,
        package_directory: impl AsRef<Path>,
    ) -> Result<EnvironmentPluginInstallReport> {
        let PreparedEnvironmentPluginPackage {
            package,
            executable_bytes,
            executable_permissions,
            executable_digest,
            payload_material,
            payload_digest,
            signature,
        } = prepare_environment_plugin_package(package_directory)?;
        let (publisher, publisher_key_id, trust, certification_tier) =
            if let Some(signature) = &signature {
                let trusted = self.verify_adapter_package_signature(signature, &payload_digest)?;
                (
                    Some(trusted.publisher),
                    Some(trusted.key_id),
                    "publisher_signed".to_string(),
                    "publisher-authenticated-experimental".to_string(),
                )
            } else {
                (
                    None,
                    None,
                    "local_unsigned".to_string(),
                    "local-experimental".to_string(),
                )
            };
        let distribution_material =
            adapter_package_distribution_material(&payload_material, signature.as_ref())?;
        let distribution_hex = sha256_hex(&distribution_material);
        let distribution_digest = format!("sha256:{distribution_hex}");
        let executable_file = package.executable.path.clone();
        let installed = InstalledPluginManifest {
            package: package.clone(),
            payload_digest,
            distribution_digest: distribution_digest.clone(),
            executable_digest: executable_digest.clone(),
            executable_file: executable_file.clone(),
            signature,
        };

        for metadata in super::workspace_environment::registered_environment_adapter_metadata() {
            if metadata.canonical_identity == package.adapter.canonical_identity {
                return Err(Error::InvalidInput(format!(
                    "adapter identity `{}` is reserved by a built-in adapter",
                    package.adapter.canonical_identity
                )));
            }
            if let Some(selector) = package
                .adapter
                .selectors
                .iter()
                .find(|selector| metadata.selectors.contains(&selector.as_str()))
            {
                return Err(Error::InvalidInput(format!(
                    "adapter selector `{selector}` is already claimed by `{}`",
                    metadata.canonical_identity
                )));
            }
        }
        for existing in
            self.installed_environment_plugins_except(Some(&package.adapter.canonical_identity))?
        {
            if let Some(selector) = package
                .adapter
                .selectors
                .iter()
                .find(|selector| existing.manifest.adapter.selectors.contains(selector))
            {
                return Err(Error::InvalidInput(format!(
                    "adapter selector `{selector}` is already claimed by `{}`",
                    existing.manifest.adapter.canonical_identity
                )));
            }
        }
        let previous = self
            .latest_environment_plugin_registry_record(&package.adapter.canonical_identity)?
            .filter(|record| record.action == "install")
            .and_then(|record| record.distribution_digest);

        let store = self.environment_plugin_store_root();
        let packages = store.join("packages");
        fs::create_dir_all(&packages)?;
        let destination = packages.join(&distribution_hex);
        let publish_needed = if destination.exists() {
            match self.verify_environment_plugin_package(&destination, &distribution_digest) {
                Ok(_) => false,
                Err(_) => {
                    let quarantine = packages.join("quarantine");
                    fs::create_dir_all(&quarantine)?;
                    let quarantined =
                        quarantine.join(format!("{}-{:032}", distribution_hex, now_nanos()));
                    fs::rename(&destination, &quarantined)?;
                    sync_directory(&quarantine);
                    sync_directory(&packages);
                    true
                }
            }
        } else {
            true
        };
        if publish_needed {
            let staging = packages.join(format!(".install-{}-{}", std::process::id(), now_nanos()));
            fs::create_dir(&staging)?;
            let publish = (|| -> Result<()> {
                let executable_destination = staging.join(&executable_file);
                fs::write(&executable_destination, &executable_bytes)?;
                fs::set_permissions(&executable_destination, executable_permissions.clone())?;
                OpenOptions::new()
                    .read(true)
                    .open(&executable_destination)?
                    .sync_all()?;
                write_file_atomic(
                    &staging.join(INSTALLED_MANIFEST_FILE),
                    &cbor(&installed)?,
                    true,
                )?;
                sync_directory(&staging);
                match fs::rename(&staging, &destination) {
                    Ok(()) => {
                        sync_directory(&packages);
                        Ok(())
                    }
                    Err(_error) if destination.is_dir() => {
                        let _ = fs::remove_dir_all(&staging);
                        self.verify_environment_plugin_package(&destination, &distribution_digest)
                            .map(|_| ())
                    }
                    Err(error) => Err(error.into()),
                }
            })();
            if publish.is_err() {
                let _ = fs::remove_dir_all(&staging);
            }
            publish?;
        }
        self.verify_environment_plugin_package(&destination, &distribution_digest)?;

        self.append_environment_plugin_registry_record(PluginRegistryRecord {
            schema: REGISTRY_RECORD_SCHEMA.to_string(),
            canonical_identity: package.adapter.canonical_identity.clone(),
            action: "install".to_string(),
            distribution_digest: Some(distribution_digest.clone()),
            recorded_at: now_ts(),
        })?;

        Ok(EnvironmentPluginInstallReport {
            canonical_identity: package.adapter.canonical_identity,
            distribution_digest,
            executable_digest,
            package_path: destination.to_string_lossy().into_owned(),
            replaced_distribution_digest: previous,
            publisher,
            publisher_key_id,
            trust,
            certification_tier,
        })
    }

    pub fn remove_environment_adapter_plugin(
        &self,
        canonical_identity: &str,
    ) -> Result<EnvironmentPluginRemoveReport> {
        validate_plugin_identity(canonical_identity)?;
        let existing = self
            .latest_environment_plugin_registry_record(canonical_identity)?
            .filter(|record| record.action == "install")
            .and_then(|record| record.distribution_digest);
        self.append_environment_plugin_registry_record(PluginRegistryRecord {
            schema: REGISTRY_RECORD_SCHEMA.to_string(),
            canonical_identity: canonical_identity.to_string(),
            action: "remove".to_string(),
            distribution_digest: None,
            recorded_at: now_ts(),
        })?;
        Ok(EnvironmentPluginRemoveReport {
            canonical_identity: canonical_identity.to_string(),
            removed_distribution_digest: existing,
        })
    }

    pub(super) fn installed_environment_plugins(&self) -> Result<Vec<InstalledEnvironmentPlugin>> {
        self.installed_environment_plugins_except(None)
    }

    fn installed_environment_plugins_except(
        &self,
        excluded_identity: Option<&str>,
    ) -> Result<Vec<InstalledEnvironmentPlugin>> {
        let registry = self.environment_plugin_store_root().join("registry");
        if !registry.exists() {
            return Ok(Vec::new());
        }
        let mut plugins = Vec::new();
        for identity_directory in fs::read_dir(&registry)? {
            let identity_directory = identity_directory?;
            if !identity_directory.file_type()?.is_dir() {
                return Err(Error::Corrupt(format!(
                    "adapter registry contains non-directory `{}`",
                    identity_directory.path().display()
                )));
            }
            let mut records = fs::read_dir(identity_directory.path())?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            records.retain(|entry| {
                entry.file_type().is_ok_and(|kind| kind.is_file())
                    && entry.path().extension().and_then(|value| value.to_str()) == Some("cbor")
            });
            records.sort_by_key(|entry| entry.file_name());
            let Some(latest) = records.last() else {
                continue;
            };
            let record: PluginRegistryRecord = serde_cbor::from_slice(&fs::read(latest.path())?)
                .map_err(|error| {
                    Error::Corrupt(format!(
                        "cannot decode adapter registry record `{}`: {error}",
                        latest.path().display()
                    ))
                })?;
            validate_registry_record(&record, &identity_directory.file_name())?;
            if record.action == "remove" {
                continue;
            }
            if excluded_identity == Some(record.canonical_identity.as_str()) {
                continue;
            }
            let distribution_digest = record.distribution_digest.ok_or_else(|| {
                Error::Corrupt(format!(
                    "adapter install record for `{}` has no distribution digest",
                    record.canonical_identity
                ))
            })?;
            let digest_hex = distribution_digest
                .strip_prefix("sha256:")
                .ok_or_else(|| Error::Corrupt("plugin digest is not sha256".to_string()))?;
            let package_directory = self
                .environment_plugin_store_root()
                .join("packages")
                .join(digest_hex);
            plugins.push(
                self.verify_environment_plugin_package(&package_directory, &distribution_digest)?,
            );
        }
        plugins.sort_by(|left, right| {
            left.manifest
                .adapter
                .canonical_identity
                .cmp(&right.manifest.adapter.canonical_identity)
        });
        validate_plugin_catalog(&plugins)?;
        Ok(plugins)
    }

    fn latest_environment_plugin_registry_record(
        &self,
        canonical_identity: &str,
    ) -> Result<Option<PluginRegistryRecord>> {
        validate_plugin_identity(canonical_identity)?;
        let directory = self
            .environment_plugin_store_root()
            .join("registry")
            .join(sha256_hex(canonical_identity.as_bytes()));
        if !directory.exists() {
            return Ok(None);
        }
        let mut records = fs::read_dir(&directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
        records.retain(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && entry.path().extension().and_then(|value| value.to_str()) == Some("cbor")
        });
        records.sort_by_key(|entry| entry.file_name());
        let Some(latest) = records.last() else {
            return Ok(None);
        };
        let record: PluginRegistryRecord = serde_cbor::from_slice(&fs::read(latest.path())?)
            .map_err(|error| {
                Error::Corrupt(format!(
                    "cannot decode adapter registry record `{}`: {error}",
                    latest.path().display()
                ))
            })?;
        validate_registry_record(&record, directory.file_name().unwrap_or_default())?;
        if record.canonical_identity != canonical_identity {
            return Err(Error::Corrupt(format!(
                "adapter registry record for `{canonical_identity}` contains identity `{}`",
                record.canonical_identity
            )));
        }
        Ok(Some(record))
    }

    fn environment_plugin_store_root(&self) -> PathBuf {
        self.db_dir
            .join("adapter-plugins")
            .join(PLUGIN_STORE_VERSION)
    }

    fn latest_publisher_trust_record(&self, key_id: &str) -> Result<Option<PublisherTrustRecord>> {
        validate_publisher_key_id(key_id)?;
        let directory = self
            .environment_plugin_store_root()
            .join("trust")
            .join(key_id.trim_start_matches("sha256:"));
        if !directory.exists() {
            return Ok(None);
        }
        let mut records = fs::read_dir(&directory)?.collect::<std::result::Result<Vec<_>, _>>()?;
        records.retain(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_file())
                && entry.path().extension().and_then(|value| value.to_str()) == Some("cbor")
        });
        records.sort_by_key(|entry| entry.file_name());
        let Some(latest) = records.last() else {
            return Ok(None);
        };
        let record: PublisherTrustRecord = serde_cbor::from_slice(&fs::read(latest.path())?)
            .map_err(|error| {
                Error::Corrupt(format!(
                    "cannot decode publisher trust record `{}`: {error}",
                    latest.path().display()
                ))
            })?;
        validate_publisher_trust_record(&record, directory.file_name().unwrap_or_default())?;
        if record.key_id != key_id {
            return Err(Error::Corrupt(format!(
                "publisher trust record for `{key_id}` contains key `{}`",
                record.key_id
            )));
        }
        Ok(Some(record))
    }

    fn append_publisher_trust_record(&self, record: PublisherTrustRecord) -> Result<()> {
        validate_publisher_trust_record(
            &record,
            OsStr::new(record.key_id.trim_start_matches("sha256:")),
        )?;
        let directory = self
            .environment_plugin_store_root()
            .join("trust")
            .join(record.key_id.trim_start_matches("sha256:"));
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!(
            "{:032}-{:010}-{}.cbor",
            now_nanos(),
            std::process::id(),
            record.action
        ));
        write_file_atomic(&path, &cbor(&record)?, true)?;
        sync_directory(&directory);
        Ok(())
    }

    fn trusted_publisher_key(&self, key_id: &str) -> Result<PublisherTrustRecord> {
        self.latest_publisher_trust_record(key_id)?
            .filter(|record| record.action == "trust")
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "adapter publisher key `{key_id}` is not trusted in this workspace"
                ))
            })
    }

    fn verify_adapter_package_signature(
        &self,
        signature: &AdapterPackageSignature,
        payload_digest: &str,
    ) -> Result<PublisherTrustRecord> {
        validate_adapter_package_signature(signature)?;
        if signature.payload_digest != payload_digest {
            return Err(Error::InvalidInput(format!(
                "adapter signature authenticates `{}` but package payload is `{payload_digest}`",
                signature.payload_digest
            )));
        }
        let trusted = self.trusted_publisher_key(&signature.key_id)?;
        if trusted.publisher != signature.publisher {
            return Err(Error::InvalidInput(format!(
                "adapter signature publisher `{}` does not match trusted key owner `{}`",
                signature.publisher, trusted.publisher
            )));
        }
        let public_key = decode_fixed_hex::<32>(&trusted.public_key, "publisher public key")?;
        let signature_bytes = decode_fixed_hex::<64>(&signature.signature, "adapter signature")?;
        let verifying_key = VerifyingKey::from_bytes(&public_key).map_err(|error| {
            Error::InvalidInput(format!("publisher public key is invalid: {error}"))
        })?;
        let signature_value = Signature::from_bytes(&signature_bytes);
        verifying_key
            .verify_strict(
                &adapter_package_signature_message(payload_digest),
                &signature_value,
            )
            .map_err(|_| {
                Error::InvalidInput(format!(
                    "adapter package signature from `{}` is invalid",
                    signature.publisher
                ))
            })?;
        Ok(trusted)
    }

    fn append_environment_plugin_registry_record(
        &self,
        record: PluginRegistryRecord,
    ) -> Result<()> {
        let identity_hash = sha256_hex(record.canonical_identity.as_bytes());
        let directory = self
            .environment_plugin_store_root()
            .join("registry")
            .join(identity_hash);
        fs::create_dir_all(&directory)?;
        let path = directory.join(format!(
            "{:032}-{:010}-{}.cbor",
            now_nanos(),
            std::process::id(),
            record.action
        ));
        write_file_atomic(&path, &cbor(&record)?, true)?;
        sync_directory(&directory);
        Ok(())
    }

    fn verify_environment_plugin_package(
        &self,
        package_directory: &Path,
        expected_distribution_digest: &str,
    ) -> Result<InstalledEnvironmentPlugin> {
        let store = fs::canonicalize(self.environment_plugin_store_root())?;
        let package_directory = fs::canonicalize(package_directory).map_err(|error| {
            Error::Corrupt(format!(
                "installed adapter package `{}` is unavailable: {error}",
                package_directory.display()
            ))
        })?;
        if !package_directory.starts_with(&store) {
            return Err(Error::Corrupt(format!(
                "installed adapter package `{}` escapes its store",
                package_directory.display()
            )));
        }
        let manifest_path = package_directory.join(INSTALLED_MANIFEST_FILE);
        let metadata = fs::symlink_metadata(&manifest_path)?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_PACKAGE_MANIFEST_BYTES
        {
            return Err(Error::Corrupt(format!(
                "installed adapter manifest `{}` is not a bounded regular file",
                manifest_path.display()
            )));
        }
        let mut installed: InstalledPluginManifest =
            serde_cbor::from_slice(&fs::read(&manifest_path)?).map_err(|error| {
                Error::Corrupt(format!(
                    "cannot decode installed adapter manifest `{}`: {error}",
                    manifest_path.display()
                ))
            })?;
        canonicalize_and_validate_package(&mut installed.package)?;
        if installed.executable_file != installed.package.executable.path {
            return Err(Error::Corrupt(
                "installed adapter executable file disagrees with its package manifest".to_string(),
            ));
        }
        if installed.distribution_digest != expected_distribution_digest {
            return Err(Error::Corrupt(format!(
                "installed adapter package digest `{}` does not match registry digest `{expected_distribution_digest}`",
                installed.distribution_digest
            )));
        }
        let executable_path = package_directory.join(&installed.executable_file);
        let executable_metadata = fs::symlink_metadata(&executable_path)?;
        if !executable_metadata.is_file()
            || executable_metadata.file_type().is_symlink()
            || executable_metadata.len() > MAX_PLUGIN_EXECUTABLE_BYTES
        {
            return Err(Error::Corrupt(format!(
                "installed adapter executable `{}` is not a bounded regular file",
                executable_path.display()
            )));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if executable_metadata.permissions().mode() & 0o111 == 0 {
                return Err(Error::Corrupt(format!(
                    "installed adapter executable `{}` lost its executable mode",
                    executable_path.display()
                )));
            }
        }
        let executable_digest = format!("sha256:{}", sha256_hex(&fs::read(&executable_path)?));
        if executable_digest != installed.executable_digest
            || executable_digest != installed.package.executable.sha256
        {
            return Err(Error::Corrupt(format!(
                "installed adapter executable `{}` failed digest verification",
                executable_path.display()
            )));
        }
        let executable_bytes = fs::read(&executable_path)?;
        let (payload_material, payload_digest) =
            adapter_package_payload(&installed.package, &executable_bytes)?;
        if !installed.payload_digest.is_empty() && installed.payload_digest != payload_digest {
            return Err(Error::Corrupt(format!(
                "installed adapter package `{}` failed payload verification",
                package_directory.display()
            )));
        }
        let (publisher, publisher_key_id, trust, certification_tier) = if let Some(signature) =
            &installed.signature
        {
            let trusted = self
                    .verify_adapter_package_signature(signature, &payload_digest)
                    .map_err(|error| {
                        Error::Corrupt(format!(
                            "installed adapter package `{}` no longer has a valid trusted publisher: {error}",
                            package_directory.display()
                        ))
                    })?;
            (
                Some(trusted.publisher),
                Some(trusted.key_id),
                "publisher_signed".to_string(),
                "publisher-authenticated-experimental".to_string(),
            )
        } else {
            (
                None,
                None,
                "local_unsigned".to_string(),
                "local-experimental".to_string(),
            )
        };
        let distribution_material =
            adapter_package_distribution_material(&payload_material, installed.signature.as_ref())?;
        let actual_distribution = format!("sha256:{}", sha256_hex(&distribution_material));
        if actual_distribution != expected_distribution_digest {
            return Err(Error::Corrupt(format!(
                "installed adapter package `{}` failed distribution verification",
                package_directory.display()
            )));
        }
        Ok(InstalledEnvironmentPlugin {
            manifest: installed.package,
            distribution_digest: installed.distribution_digest,
            executable_digest,
            executable_path,
            publisher,
            publisher_key_id,
            trust,
            certification_tier,
        })
    }
}

impl Trail {
    pub(super) fn environment_plugin_for_selector(
        &self,
        selector: &str,
    ) -> Result<Option<InstalledEnvironmentPlugin>> {
        Ok(self
            .installed_environment_plugins()?
            .into_iter()
            .find(|plugin| {
                plugin
                    .manifest
                    .adapter
                    .selectors
                    .iter()
                    .any(|candidate| candidate == selector)
            }))
    }

    pub(super) fn discover_environment_plugin_component(
        &self,
        plugin: &InstalledEnvironmentPlugin,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<Option<EnvironmentDiscoveredComponentReport>> {
        ensure_environment_plugin_supports_current_host(plugin)?;
        let protocol = selected_environment_plugin_protocol(plugin)?;
        let selected = self.select_environment_plugin_files(plugin, source_root, component_root)?;
        let request_id = plugin_request_id(plugin, source_root, component_root, "discover", None);
        let request = AdapterRequest {
            protocol: protocol.to_string(),
            request_id: request_id.clone(),
            adapter_identity: plugin.manifest.adapter.canonical_identity.clone(),
            distribution_digest: plugin.distribution_digest.clone(),
            host: AdapterHost {
                operating_system: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
            },
            source_root: source_root.0.clone(),
            operation: AdapterOperation::Discover {
                component_root: component_root.to_string(),
                files: selected.protocol_files,
            },
        };
        match self.invoke_environment_plugin(plugin, &request)? {
            AdapterResult::Discovered { component: None } => Ok(None),
            AdapterResult::Discovered {
                component: Some(component),
            } => {
                validate_discovered_plugin_component(plugin, &component)?;
                Ok(Some(EnvironmentDiscoveredComponentReport {
                    component_id: component.component_id,
                    component_root: component_root.to_string(),
                    kind: component.kind,
                    adapter_identity: plugin.manifest.adapter.canonical_identity.clone(),
                }))
            }
            _ => Err(Error::InvalidInput(format!(
                "adapter `{}` returned a non-discovery response to request `{request_id}`",
                plugin.manifest.adapter.canonical_identity
            ))),
        }
    }

    pub(super) fn plan_environment_plugin_component(
        &self,
        plugin: &InstalledEnvironmentPlugin,
        source_root: &ObjectId,
        component_root: &str,
        component_id: &str,
    ) -> Result<WorkspaceEnvironmentPlan> {
        ensure_environment_plugin_supports_current_host(plugin)?;
        let protocol = selected_environment_plugin_protocol(plugin)?;
        let selected = self.select_environment_plugin_files(plugin, source_root, component_root)?;
        let request_id = plugin_request_id(
            plugin,
            source_root,
            component_root,
            "plan",
            Some(component_id),
        );
        let request = AdapterRequest {
            protocol: protocol.to_string(),
            request_id: request_id.clone(),
            adapter_identity: plugin.manifest.adapter.canonical_identity.clone(),
            distribution_digest: plugin.distribution_digest.clone(),
            host: AdapterHost {
                operating_system: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
            },
            source_root: source_root.0.clone(),
            operation: AdapterOperation::Plan {
                component_id: component_id.to_string(),
                component_root: component_root.to_string(),
                files: selected.protocol_files,
            },
        };
        match (protocol, self.invoke_environment_plugin(plugin, &request)?) {
            (PROTOCOL_V1, AdapterResult::Planned { plan }) => self
                .normalize_environment_plugin_plan(
                    plugin,
                    component_root,
                    component_id,
                    selected.entries,
                    protocol,
                    plan.into(),
                ),
            (PROTOCOL_V2, AdapterResult::PlannedV2 { plan }) => self
                .normalize_environment_plugin_plan(
                    plugin,
                    component_root,
                    component_id,
                    selected.entries,
                    protocol,
                    ProposedPluginPlan::from_v2(plan)?,
                ),
            _ => Err(Error::InvalidInput(format!(
                "adapter `{}` returned a non-plan response to request `{request_id}`",
                plugin.manifest.adapter.canonical_identity
            ))),
        }
    }

    fn select_environment_plugin_files(
        &self,
        plugin: &InstalledEnvironmentPlugin,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<SelectedPluginFiles> {
        let component_root = normalize_plugin_component_root(component_root)?;
        let patterns = compile_plugin_patterns(&plugin.manifest.permissions)?;
        let prefix = if component_root.is_empty() {
            String::new()
        } else {
            format!("{component_root}/")
        };
        let mut entries = BTreeMap::new();
        let mut total_bytes = 0u64;
        self.for_each_root_file_chunk(source_root, 1024, |chunk| {
            for (repository_path, entry) in chunk {
                let relative = if prefix.is_empty() {
                    repository_path.as_str()
                } else if let Some(relative) = repository_path.strip_prefix(&prefix) {
                    relative
                } else {
                    continue;
                };
                if !patterns.is_match(relative) {
                    continue;
                }
                if entries.len() >= plugin.manifest.permissions.max_input_files as usize {
                    return Err(Error::InvalidInput(format!(
                        "adapter `{}` input selection exceeds {} files",
                        plugin.manifest.adapter.canonical_identity,
                        plugin.manifest.permissions.max_input_files
                    )));
                }
                total_bytes = total_bytes.checked_add(entry.size_bytes).ok_or_else(|| {
                    Error::InvalidInput("adapter input byte count overflowed".to_string())
                })?;
                if total_bytes > plugin.manifest.permissions.max_input_bytes {
                    return Err(Error::InvalidInput(format!(
                        "adapter `{}` input selection exceeds {} bytes",
                        plugin.manifest.adapter.canonical_identity,
                        plugin.manifest.permissions.max_input_bytes
                    )));
                }
                entries.insert(relative.to_string(), (repository_path, entry));
            }
            Ok(())
        })?;
        let mut protocol_files = Vec::with_capacity(entries.len());
        for (relative, (_, entry)) in &entries {
            let content = fs::read(self.project_entry_file(entry)?)?;
            if content.len() as u64 != entry.size_bytes {
                return Err(Error::Corrupt(format!(
                    "pinned adapter input `{relative}` has inconsistent size"
                )));
            }
            protocol_files.push(PinnedFile {
                path: relative.clone(),
                content_hash: entry.content_hash.clone(),
                executable: entry.executable,
                content,
            });
        }
        Ok(SelectedPluginFiles {
            protocol_files,
            entries,
        })
    }

    fn invoke_environment_plugin(
        &self,
        plugin: &InstalledEnvironmentPlugin,
        request: &AdapterRequest,
    ) -> Result<AdapterResult> {
        let mut request_bytes = Vec::new();
        write_frame(&mut request_bytes, request, MAX_FRAME_BYTES)
            .map_err(|error| Error::Serialization(error.to_string()))?;
        let sandbox_parent = workspace_environment_temporary_parent()?;
        let sandbox = tempfile::Builder::new()
            .prefix("trail-adapter-plugin-")
            .tempdir_in(&sandbox_parent)?;
        let sandbox_root = fs::canonicalize(sandbox.path())?;
        let executable_name = plugin
            .executable_path
            .file_name()
            .ok_or_else(|| Error::Corrupt("installed adapter has no file name".to_string()))?;
        let executable = sandbox_root.join(executable_name);
        fs::copy(&plugin.executable_path, &executable)?;
        fs::set_permissions(
            &executable,
            fs::metadata(&plugin.executable_path)?.permissions(),
        )?;
        let executable = fs::canonicalize(executable)?;
        let copied_digest = format!("sha256:{}", sha256_hex(&fs::read(&executable)?));
        if copied_digest != plugin.executable_digest {
            return Err(Error::Corrupt(format!(
                "staged adapter executable for `{}` failed digest verification",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        let home = sandbox_root.join("home");
        let temporary = sandbox_root.join("tmp");
        fs::create_dir(&home)?;
        fs::create_dir(&temporary)?;
        let home = fs::canonicalize(home)?;
        let temporary = fs::canonicalize(temporary)?;
        let (launcher, launcher_args) =
            sandboxed_plugin_launcher(&sandbox_root, &home, &temporary, &executable)?;

        let mut command = Command::new(launcher);
        command
            .args(launcher_args)
            .current_dir(&sandbox_root)
            .env_clear()
            .env("HOME", &home)
            .env("TMPDIR", &temporary)
            .env("TMP", &temporary)
            .env("TEMP", &temporary)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        if let Some(system_root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", system_root);
        }
        configure_plugin_resource_limits(&mut command, plugin.manifest.permissions.timeout_ms)?;
        let mut child = command.spawn().map_err(|error| {
            Error::InvalidInput(format!(
                "cannot launch adapter plugin `{}`: {error}",
                plugin.manifest.adapter.canonical_identity
            ))
        })?;
        let mut stdin = child.stdin.take().ok_or_else(|| {
            Error::Corrupt("adapter plugin stdin pipe was unavailable".to_string())
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            Error::Corrupt("adapter plugin stdout pipe was unavailable".to_string())
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            Error::Corrupt("adapter plugin stderr pipe was unavailable".to_string())
        })?;
        let writer = thread::spawn(move || -> std::io::Result<()> {
            stdin.write_all(&request_bytes)?;
            stdin.flush()
        });
        let response_limit = usize::try_from(plugin.manifest.permissions.max_response_bytes)
            .unwrap_or(MAX_FRAME_BYTES)
            .min(MAX_FRAME_BYTES);
        let stdout_reader = spawn_bounded_reader(stdout, response_limit.saturating_add(4));
        let stderr_reader = spawn_bounded_reader(stderr, 64 * 1024);
        let status = wait_for_plugin(
            &mut child,
            Duration::from_millis(plugin.manifest.permissions.timeout_ms),
        );
        let writer_result = writer
            .join()
            .map_err(|_| Error::Corrupt("adapter plugin stdin writer panicked".to_string()))?;
        let stdout = stdout_reader
            .join()
            .map_err(|_| Error::Corrupt("adapter plugin stdout reader panicked".to_string()))??;
        let stderr = stderr_reader
            .join()
            .map_err(|_| Error::Corrupt("adapter plugin stderr reader panicked".to_string()))??;
        let status = status?;
        writer_result.map_err(Error::Io)?;
        if stdout.overflow {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` response exceeded {response_limit} bytes",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        if stderr.overflow {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` diagnostic output exceeded 65536 bytes",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        if !status.success() {
            let diagnostic = redact_sensitive_text(&String::from_utf8_lossy(&stderr.bytes));
            return Err(Error::InvalidInput(format!(
                "adapter `{}` exited with {status}: {}",
                plugin.manifest.adapter.canonical_identity,
                diagnostic.trim()
            )));
        }
        let mut cursor = Cursor::new(&stdout.bytes);
        let response: AdapterResponse = read_frame(&mut cursor, response_limit)
            .map_err(|error| Error::InvalidInput(format!("invalid adapter response: {error}")))?;
        if cursor.position() != stdout.bytes.len() as u64 {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` wrote trailing bytes after its response frame",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        if response.protocol != request.protocol || response.request_id != request.request_id {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` returned a mismatched protocol or request ID",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        match response.result {
            AdapterResult::Error { code, message } => Err(Error::InvalidInput(format!(
                "adapter `{}` rejected request with {}: {}",
                plugin.manifest.adapter.canonical_identity,
                redact_sensitive_text(&code),
                redact_sensitive_text(&message)
            ))),
            result => Ok(result),
        }
    }

    fn normalize_environment_plugin_caches(
        &self,
        plugin: &InstalledEnvironmentPlugin,
        protocol: &str,
        caches: &[AdapterCache],
        has_staging_action: bool,
    ) -> Result<(Vec<WorkspaceEnvironmentCache>, BTreeMap<String, String>)> {
        if caches.is_empty() {
            return Ok((Vec::new(), BTreeMap::new()));
        }
        if protocol != PROTOCOL_V2 || !has_staging_action {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` may use host caches only from a protocol-v2 staging action",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        if caches.len() > 16 {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` returned more than sixteen cache declarations",
                plugin.manifest.adapter.canonical_identity
            )));
        }

        let mut proposed = caches.to_vec();
        proposed.sort_by(|left, right| left.name.cmp(&right.name));
        let mut names = BTreeSet::new();
        let mut environment = BTreeMap::new();
        let mut normalized = Vec::with_capacity(proposed.len());
        for cache in proposed {
            if !names.insert(cache.name.clone()) {
                return Err(Error::InvalidInput(format!(
                    "adapter `{}` repeats cache declaration `{}`",
                    plugin.manifest.adapter.canonical_identity, cache.name
                )));
            }
            if cache.access != AdapterCacheAccess::HostExclusive {
                return Err(Error::InvalidInput(format!(
                    "adapter `{}` cache `{}` requests tool-concurrent access without independent cache certification; use host_exclusive",
                    plugin.manifest.adapter.canonical_identity, cache.name
                )));
            }
            if cache.compatibility.len() > 24
                || cache
                    .compatibility
                    .keys()
                    .any(|name| name.starts_with("trail."))
            {
                return Err(Error::InvalidInput(format!(
                    "adapter `{}` cache `{}` has too many or host-reserved compatibility dimensions",
                    plugin.manifest.adapter.canonical_identity, cache.name
                )));
            }
            if cache.environment.is_empty() || cache.environment.len() > 16 {
                return Err(Error::InvalidInput(format!(
                    "adapter `{}` cache `{}` must declare between one and sixteen environment bindings",
                    plugin.manifest.adapter.canonical_identity, cache.name
                )));
            }

            let mut compatibility = cache.compatibility;
            compatibility.insert(
                "trail.adapter_distribution".to_string(),
                plugin.distribution_digest.clone(),
            );
            compatibility.insert("trail.protocol".to_string(), protocol.to_string());
            compatibility.insert(
                "trail.operating_system".to_string(),
                std::env::consts::OS.to_string(),
            );
            compatibility.insert(
                "trail.architecture".to_string(),
                std::env::consts::ARCH.to_string(),
            );
            let protocol = match cache.protocol {
                AdapterCacheProtocol::ContentStore => {
                    WorkspaceEnvironmentCacheProtocol::ContentStore
                }
                AdapterCacheProtocol::CompilerCache => {
                    WorkspaceEnvironmentCacheProtocol::CompilerCache
                }
                AdapterCacheProtocol::LockedIndex => WorkspaceEnvironmentCacheProtocol::LockedIndex,
            };
            let declared = self.declare_workspace_environment_cache(
                &plugin.manifest.adapter.canonical_identity,
                &cache.name,
                protocol,
                WorkspaceEnvironmentCacheAccess::HostExclusive,
                compatibility,
            )?;
            for (name, subpath) in cache.environment {
                super::workspace_recipe::validate_recipe_environment(
                    &name,
                    &subpath,
                    &plugin.manifest.adapter.canonical_identity,
                )?;
                let relative = normalize_plugin_path_allow_root(&subpath)?;
                let value = if relative.is_empty() {
                    declared.storage_path.clone()
                } else {
                    declared.storage_path.join(&relative)
                };
                if environment
                    .insert(name.clone(), value.to_string_lossy().into_owned())
                    .is_some()
                {
                    return Err(Error::InvalidInput(format!(
                        "adapter `{}` binds cache environment variable `{name}` more than once",
                        plugin.manifest.adapter.canonical_identity
                    )));
                }
            }
            normalized.push(declared);
        }
        Ok((normalized, environment))
    }

    fn normalize_environment_plugin_plan(
        &self,
        plugin: &InstalledEnvironmentPlugin,
        component_root: &str,
        expected_component_id: &str,
        entries: BTreeMap<String, (String, FileEntry)>,
        protocol: &str,
        plan: ProposedPluginPlan,
    ) -> Result<WorkspaceEnvironmentPlan> {
        validate_plugin_component_id(&plan.component_id)?;
        if plan.component_id != expected_component_id {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` planned component `{}` instead of requested `{expected_component_id}`",
                plugin.manifest.adapter.canonical_identity, plan.component_id
            )));
        }
        if plan.kind != plugin.manifest.adapter.kind {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` planned kind `{}` but its package declares `{}`",
                plugin.manifest.adapter.canonical_identity, plan.kind, plugin.manifest.adapter.kind
            )));
        }
        let declared_inputs = plan
            .identity_inputs
            .iter()
            .map(|path| normalize_relative_path(path))
            .collect::<Result<BTreeSet<_>>>()?;
        if declared_inputs.len() != plan.identity_inputs.len() {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` returned duplicate identity inputs",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        let supplied_inputs = entries.keys().cloned().collect::<BTreeSet<_>>();
        if declared_inputs != supplied_inputs {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` must key every pinned file it was allowed to inspect",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        if supplied_inputs.is_empty() {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` cannot plan a component without pinned identity inputs",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        if plan.semantic_inputs.len() > 256 {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` returned more than 256 semantic inputs",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        let mut semantic_bytes = 0usize;
        for (name, value) in &plan.semantic_inputs {
            semantic_bytes = semantic_bytes
                .checked_add(name.len() + value.len())
                .ok_or_else(|| Error::InvalidInput("semantic input size overflowed".to_string()))?;
            if name.is_empty()
                || name.len() > 256
                || !name.chars().all(|character| {
                    character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ':')
                })
                || value.len() > 128 * 1024
                || value.contains('\0')
                || contains_sensitive_text(name)
                || contains_sensitive_text(value)
            {
                return Err(Error::InvalidInput(format!(
                    "adapter `{}` returned invalid or sensitive semantic input `{name}`",
                    plugin.manifest.adapter.canonical_identity
                )));
            }
        }
        if semantic_bytes > 1024 * 1024 {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` semantic inputs exceed one MiB",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        let component_root = normalize_plugin_component_root(component_root)?;
        match protocol {
            PROTOCOL_V1 if plan.command.is_none() || !plan.mounted_commands.is_empty() => {
                return Err(Error::InvalidInput(format!(
                    "adapter `{}` returned protocol-v2 actions in a v1 plan",
                    plugin.manifest.adapter.canonical_identity
                )));
            }
            PROTOCOL_V1 => {}
            PROTOCOL_V2
                if plan.command.is_none()
                    && plan.mounted_commands.is_empty()
                    && plan.external_artifacts.is_empty()
                    && plan.runtime_resources.is_empty() =>
            {
                return Err(Error::InvalidInput(format!(
                    "adapter `{}` returned a v2 filesystem plan without an action",
                    plugin.manifest.adapter.canonical_identity
                )));
            }
            PROTOCOL_V2 if plan.mounted_commands.len() > 8 => {
                return Err(Error::InvalidInput(format!(
                    "adapter `{}` returned more than eight mounted initialization actions",
                    plugin.manifest.adapter.canonical_identity
                )));
            }
            PROTOCOL_V2 => {}
            other => {
                return Err(Error::InvalidInput(format!(
                    "adapter `{}` used unsupported protocol `{other}`",
                    plugin.manifest.adapter.canonical_identity
                )));
            }
        }

        let cache_contract = serde_json::to_string(&plan.caches)?;
        let (normalized_caches, cache_environment) = self.normalize_environment_plugin_caches(
            plugin,
            protocol,
            &plan.caches,
            plan.command.is_some(),
        )?;
        let cache_names = normalized_caches
            .iter()
            .map(|cache| cache.name.clone())
            .collect::<Vec<_>>();

        let normalize_command = |command: &trail_environment_adapter_sdk::AdapterCommand,
                                 mounted: bool|
         -> Result<(WorkspaceEnvironmentCommand, String)> {
            if command.program.contains('/')
                || command.program.contains('\\')
                || super::workspace_recipe::is_shell_program(&command.program)
                || command.args.len() > 4096
                || command.args.iter().any(|argument| {
                    argument.len() > 128 * 1024
                        || argument.contains('\0')
                        || contains_sensitive_text(argument)
                })
            {
                return Err(Error::InvalidInput(format!(
                    "adapter `{}` proposed a shell, path-qualified executable, or invalid argument",
                    plugin.manifest.adapter.canonical_identity
                )));
            }
            let tool =
                super::workspace_environment::resolve_workspace_tool_executable(&command.program)?;
            super::workspace_recipe::validate_recipe_tool_path(
                self,
                &tool.path,
                &plan.component_id,
            )?;
            for (name, value) in &command.environment {
                super::workspace_recipe::validate_recipe_environment(
                    name,
                    value,
                    &plan.component_id,
                )?;
            }
            let mut environment = command.environment.clone();
            if !mounted {
                for (name, value) in &cache_environment {
                    if environment.insert(name.clone(), value.clone()).is_some() {
                        return Err(Error::InvalidInput(format!(
                            "adapter `{}` binds cache environment variable `{name}` more than once",
                            plugin.manifest.adapter.canonical_identity
                        )));
                    }
                }
            }
            let working_relative = normalize_plugin_path_allow_root(&command.working_directory)?;
            let working_repository_path = join_plugin_path(&component_root, &working_relative);
            let working_directory = if mounted {
                working_repository_path
            } else if working_repository_path.is_empty() {
                "project".to_string()
            } else {
                format!("project/{working_repository_path}")
            };
            Ok((
                WorkspaceEnvironmentCommand {
                    program: command.program.clone(),
                    resolved_program: tool.path,
                    executable_identity: tool.identity,
                    args: command.args.clone(),
                    working_directory,
                    environment,
                    remove_environment: Vec::new(),
                    cache_names: if mounted {
                        Vec::new()
                    } else {
                        cache_names.clone()
                    },
                },
                working_relative,
            ))
        };
        let normalized_command = plan
            .command
            .as_ref()
            .map(|command| normalize_command(command, false))
            .transpose()?;
        let normalized_mounted_commands = plan
            .mounted_commands
            .iter()
            .map(|command| normalize_command(command, true))
            .collect::<Result<Vec<_>>>()?;
        let mut external_artifacts = plan
            .external_artifacts
            .iter()
            .map(|artifact| WorkspaceEnvironmentExternalArtifact {
                name: artifact.name.clone(),
                artifact_type: artifact.artifact_type.clone(),
                provider: artifact.provider.clone(),
                reference: artifact.reference.clone(),
                digest: artifact.digest.clone(),
                platform: artifact.platform.clone(),
                cleanup_owner: artifact.cleanup_owner.clone(),
            })
            .collect::<Vec<_>>();
        external_artifacts.sort_by(|left, right| left.name.cmp(&right.name));
        let mut runtime_resources = plan
            .runtime_resources
            .iter()
            .map(|resource| WorkspaceEnvironmentRuntimeResource {
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
                    .map(|secret| WorkspaceEnvironmentSecretReference {
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
            })
            .collect::<Vec<_>>();
        runtime_resources.sort_by(|left, right| left.name.cmp(&right.name));
        let working_repository_path = normalized_command
            .as_ref()
            .map(|(_, working_relative)| join_plugin_path(&component_root, working_relative))
            .unwrap_or_else(|| component_root.clone());
        if plan.external_artifacts.is_empty()
            && (plan.outputs.is_empty() || plan.outputs.len() > 32)
        {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` must propose between one and 32 outputs",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        if !plan.external_artifacts.is_empty() && !plan.outputs.is_empty() {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` cannot mix external artifacts with filesystem outputs",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        let mut output_names = BTreeSet::new();
        let mut output_paths = Vec::<(String, String)>::new();
        let mut mount_paths = Vec::<(String, String)>::new();
        let mut outputs = Vec::with_capacity(plan.outputs.len());
        for output in &plan.outputs {
            if !output.create_if_missing {
                return Err(Error::InvalidInput(format!(
                    "adapter `{}` must allow Trail to create output `{}` before sandbox execution",
                    plugin.manifest.adapter.canonical_identity, output.name
                )));
            }
            validate_plugin_output_name(&output.name)?;
            if !output_names.insert(output.name.clone()) {
                return Err(Error::InvalidInput(format!(
                    "adapter `{}` proposed duplicate output `{}`",
                    plugin.manifest.adapter.canonical_identity, output.name
                )));
            }
            let source = normalize_relative_path(&output.source)?;
            let target = normalize_relative_path(&output.target)?;
            let output_repository_path = join_plugin_path(&working_repository_path, &source);
            let mount_path = join_plugin_path(&component_root, &target);
            for (other_name, other_path) in &output_paths {
                if plugin_paths_overlap(&output_repository_path, other_path) {
                    return Err(Error::InvalidInput(format!(
                        "adapter `{}` output `{}` overlaps output `{other_name}`",
                        plugin.manifest.adapter.canonical_identity, output.name
                    )));
                }
            }
            for (other_name, other_path) in &mount_paths {
                if plugin_paths_overlap(&mount_path, other_path) {
                    return Err(Error::InvalidInput(format!(
                        "adapter `{}` mount `{}` overlaps output `{other_name}`",
                        plugin.manifest.adapter.canonical_identity, output.name
                    )));
                }
            }
            for (relative, (repository_path, _)) in &entries {
                if plugin_paths_overlap(&output_repository_path, repository_path)
                    || plugin_paths_overlap(&mount_path, repository_path)
                {
                    return Err(Error::InvalidInput(format!(
                        "adapter `{}` output `{}` overlaps pinned input `{relative}`",
                        plugin.manifest.adapter.canonical_identity, output.name
                    )));
                }
            }
            output_paths.push((output.name.clone(), output_repository_path.clone()));
            mount_paths.push((output.name.clone(), mount_path.clone()));
            outputs.push(WorkspaceEnvironmentOutput {
                name: output.name.clone(),
                output_path: format!("project/{output_repository_path}"),
                mount_path,
                policy: match output.policy {
                    AdapterOutputPolicy::ImmutableSeedPrivate => {
                        WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate
                    }
                    AdapterOutputPolicy::WritablePrivate => {
                        WorkspaceEnvironmentOutputPolicy::WritablePrivate
                    }
                },
                create_if_missing: output.create_if_missing,
            });
        }
        if !normalized_mounted_commands.is_empty()
            && outputs
                .iter()
                .any(|output| output.policy != WorkspaceEnvironmentOutputPolicy::WritablePrivate)
        {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` may use mounted initialization only with writable-private outputs",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        let output_contract = serde_json::to_string(
            &outputs
                .iter()
                .map(|output| {
                    (
                        &output.name,
                        &output.output_path,
                        &output.mount_path,
                        output.policy.as_str(),
                    )
                })
                .collect::<Vec<_>>(),
        )?;
        let external_artifact_contract =
            super::workspace_environment::workspace_external_artifacts_identity(
                &external_artifacts,
            )?;
        let runtime_resource_contract =
            super::workspace_environment::workspace_runtime_resources_identity(&runtime_resources)?;
        let mut layer_inputs = BTreeMap::from([
            ("component_id".to_string(), plan.component_id.clone()),
            ("component_root".to_string(), component_root.clone()),
            ("protocol".to_string(), protocol.to_string()),
            (
                "adapter_implementation".to_string(),
                plugin.manifest.adapter.implementation_version.clone(),
            ),
            (
                "adapter_distribution_digest".to_string(),
                plugin.distribution_digest.clone(),
            ),
            (
                "adapter_executable_digest".to_string(),
                plugin.executable_digest.clone(),
            ),
            (
                "staging_action".to_string(),
                serde_json::to_string(
                    &plan.command.as_ref().zip(
                        normalized_command
                            .as_ref()
                            .map(|(_, working_relative)| working_relative),
                    ),
                )?,
            ),
            (
                "mounted_actions".to_string(),
                serde_json::to_string(
                    &plan
                        .mounted_commands
                        .iter()
                        .zip(
                            normalized_mounted_commands
                                .iter()
                                .map(|(_, working_relative)| working_relative),
                        )
                        .collect::<Vec<_>>(),
                )?,
            ),
            ("output_contract".to_string(), output_contract),
            (
                "capability_contract".to_string(),
                if normalized_mounted_commands.is_empty() {
                    "plugin-plan:bounded-pinned-bytes;action:staging;fs-read:declared-inputs;fs-write:declared-outputs+isolated-home+tmp;process:exact-host-resolved-executable;child-exec:deny;network:deny;shell:deny;scripts:deny;secrets:deny"
                        .to_string()
                } else {
                    "plugin-plan:bounded-pinned-bytes;action:staging+mounted-candidate;mount-authority:host-only;fs-read:declared-inputs;fs-write:declared-writable-private-outputs+isolated-home+tmp;source-write:deny;process:exact-host-resolved-executable;child-exec:deny;network:deny;shell:deny;scripts:deny;secrets:deny"
                        .to_string()
                },
            ),
        ]);
        if !external_artifacts.is_empty() {
            layer_inputs.insert(
                "external_artifact_contract".to_string(),
                external_artifact_contract,
            );
            layer_inputs.insert(
                "capability_contract".to_string(),
                if runtime_resources
                    .iter()
                    .any(|resource| !resource.secrets.is_empty())
                {
                    "plugin-plan:bounded-pinned-bytes;action:none;filesystem:none;process:none;network:none;secrets:opaque-reference-only;secret-resolution:host-runtime-file-handle;authority:external-identity+runtime-declaration-only"
                        .to_string()
                } else {
                    "plugin-plan:bounded-pinned-bytes;action:none;filesystem:none;process:none;network:none;secrets:none;authority:external-identity-only"
                        .to_string()
                },
            );
        }
        if !runtime_resources.is_empty() {
            layer_inputs.insert(
                "runtime_resource_contract".to_string(),
                runtime_resource_contract,
            );
        }
        if !plan.caches.is_empty() {
            layer_inputs.insert("cache_contract".to_string(), cache_contract);
        }
        if protocol == PROTOCOL_V1 {
            let command = plan.command.as_ref().ok_or_else(|| {
                Error::Corrupt("validated protocol-v1 plugin plan lost its command".to_string())
            })?;
            let working_relative = normalized_command
                .as_ref()
                .map(|(_, working_relative)| working_relative)
                .ok_or_else(|| {
                    Error::Corrupt(
                        "validated protocol-v1 plugin plan lost its working directory".to_string(),
                    )
                })?;
            layer_inputs.remove("staging_action");
            layer_inputs.remove("mounted_actions");
            layer_inputs.insert(
                "command".to_string(),
                serde_json::to_string(&(
                    &command.program,
                    &command.args,
                    working_relative,
                    &command.environment,
                ))?,
            );
            layer_inputs.insert(
                "capability_contract".to_string(),
                "plugin-plan:bounded-pinned-bytes;fs-read:declared-inputs;fs-write:declared-outputs+isolated-home+tmp;process:exact-host-resolved-executable;child-exec:deny;network:deny;shell:deny;scripts:deny;secrets:deny"
                    .to_string(),
            );
        }
        let mounted_commands = normalized_mounted_commands
            .iter()
            .map(|(command, _)| command.clone())
            .collect::<Vec<_>>();
        if !mounted_commands.is_empty() {
            layer_inputs.insert(
                "mounted_action".to_string(),
                super::workspace_environment::workspace_mounted_commands_identity(
                    &mounted_commands,
                )?,
            );
        }
        for (name, value) in plan.semantic_inputs {
            layer_inputs.insert(format!("semantic:{name}"), value);
        }
        let mut inputs = Vec::with_capacity(entries.len());
        for (relative, (repository_path, entry)) in entries {
            layer_inputs.insert(
                format!("input:{repository_path}"),
                entry.content_hash.clone(),
            );
            inputs.push(WorkspaceEnvironmentInput {
                source_path: repository_path.clone(),
                staging_path: format!("project/{repository_path}"),
                entry,
            });
            let _ = relative;
        }
        let portability_scope = match plan.portability {
            AdapterPortability::Host => "plugin-tool-host",
            AdapterPortability::Platform => "plugin-tool-platform",
        };
        if plan.stale_reason.trim().is_empty()
            || plan.stale_reason.len() > 4096
            || contains_sensitive_text(&plan.stale_reason)
        {
            return Err(Error::InvalidInput(format!(
                "adapter `{}` returned an invalid stale explanation",
                plugin.manifest.adapter.canonical_identity
            )));
        }
        let mut tool_versions = BTreeMap::new();
        if let Some((command, _)) = &normalized_command {
            let name = if protocol == PROTOCOL_V1 {
                format!("executable:{}", command.program)
            } else {
                format!("staging-executable:{}", command.program)
            };
            tool_versions.insert(name, command.executable_identity.clone());
        }
        for (index, (command, _)) in normalized_mounted_commands.iter().enumerate() {
            tool_versions.insert(
                format!("mounted-executable:{index}:{}", command.program),
                command.executable_identity.clone(),
            );
        }
        let command = normalized_command.map(|(command, _)| command);
        let has_plugin_caches = !normalized_caches.is_empty();
        Ok(WorkspaceEnvironmentPlan {
            component_id: plan.component_id,
            adapter_identity: plugin.manifest.adapter.canonical_identity.clone(),
            adapter_version: 1,
            implementation_version: plugin.manifest.adapter.implementation_version.clone(),
            distribution_digest: plugin.distribution_digest.clone(),
            kind: plan.kind,
            dependencies: plan.dependencies,
            resolved_dependencies: Vec::new(),
            layer_key: WorkspaceLayerKeyV1 {
                kind: plugin.manifest.adapter.kind.clone(),
                adapter: plugin.manifest.adapter.layer_adapter_name.clone(),
                adapter_version: 1,
                inputs: layer_inputs,
                tool_versions,
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: portability_scope.to_string(),
                strategy: if !external_artifacts.is_empty() {
                    "isolated-plugin-external-artifact-plan-v2"
                } else if protocol == PROTOCOL_V1 {
                    "isolated-plugin-plan-v1"
                } else if mounted_commands.is_empty() {
                    "isolated-plugin-plan-v2"
                } else {
                    "isolated-plugin-mounted-candidate-v2"
                }
                .to_string(),
            },
            inputs,
            source_projection: None,
            pre_commands: Vec::new(),
            command,
            mounted_commands,
            caches: normalized_caches,
            external_artifacts,
            runtime_resources,
            sandbox_policy: if protocol == PROTOCOL_V2 && !normalized_mounted_commands.is_empty() {
                WorkspaceEnvironmentSandboxPolicy::RestrictedPluginMounted
            } else if protocol == PROTOCOL_V2 && has_plugin_caches {
                WorkspaceEnvironmentSandboxPolicy::RestrictedPluginStaging
            } else {
                WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe
            },
            outputs,
            stale_reason: plan.stale_reason,
        })
    }
}

fn spawn_bounded_reader(
    mut reader: impl Read + Send + 'static,
    maximum: usize,
) -> thread::JoinHandle<std::io::Result<BoundedRead>> {
    thread::spawn(move || {
        let mut bytes = Vec::with_capacity(maximum.min(64 * 1024));
        let mut overflow = false;
        let mut buffer = [0u8; 16 * 1024];
        loop {
            let read = reader.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            let remaining = maximum.saturating_sub(bytes.len());
            let retained = remaining.min(read);
            bytes.extend_from_slice(&buffer[..retained]);
            if retained != read {
                overflow = true;
            }
        }
        Ok(BoundedRead { bytes, overflow })
    })
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn configure_plugin_resource_limits(command: &mut Command, timeout_ms: u64) -> Result<()> {
    use std::os::unix::process::CommandExt;

    #[cfg(target_os = "linux")]
    type RlimitResource = libc::__rlimit_resource_t;
    #[cfg(target_os = "macos")]
    type RlimitResource = libc::c_int;

    let cpu_seconds = timeout_ms.div_ceil(1000).saturating_add(1);
    // SAFETY: the closure performs only async-signal-safe setrlimit calls and
    // constructs errors from the already-captured errno before exec.
    unsafe {
        command.pre_exec(move || {
            fn set(
                name: &str,
                resource: RlimitResource,
                soft: u64,
                hard: u64,
            ) -> std::io::Result<()> {
                let limit = libc::rlimit {
                    rlim_cur: soft as libc::rlim_t,
                    rlim_max: hard as libc::rlim_t,
                };
                // SAFETY: `limit` is initialized for the requested resource.
                if unsafe { libc::setrlimit(resource, &limit) } != 0 {
                    let source = std::io::Error::last_os_error();
                    Err(std::io::Error::new(
                        source.kind(),
                        format!("cannot set plugin {name} resource limit: {source}"),
                    ))
                } else {
                    Ok(())
                }
            }

            #[cfg(target_os = "linux")]
            set(
                "address-space",
                libc::RLIMIT_AS,
                512 * 1024 * 1024,
                512 * 1024 * 1024,
            )?;
            set("cpu", libc::RLIMIT_CPU, cpu_seconds, cpu_seconds)?;
            set(
                "file-size",
                libc::RLIMIT_FSIZE,
                16 * 1024 * 1024,
                16 * 1024 * 1024,
            )?;
            set("open-files", libc::RLIMIT_NOFILE, 64, 64)?;
            set("core", libc::RLIMIT_CORE, 0, 0)?;
            Ok(())
        });
    }
    Ok(())
}

#[cfg(windows)]
fn configure_plugin_resource_limits(_command: &mut Command, _timeout_ms: u64) -> Result<()> {
    // The hidden Windows sandbox helper applies the process-memory limit to
    // zero-output invocations (the adapter-plugin profile) on its Job Object.
    Ok(())
}

#[cfg(all(unix, not(any(target_os = "linux", target_os = "macos"))))]
fn configure_plugin_resource_limits(_command: &mut Command, _timeout_ms: u64) -> Result<()> {
    Err(Error::InvalidInput(
        "adapter plugin resource limits are unavailable on this Unix platform".to_string(),
    ))
}

#[cfg(not(any(unix, windows)))]
fn configure_plugin_resource_limits(_command: &mut Command, _timeout_ms: u64) -> Result<()> {
    Err(Error::InvalidInput(
        "adapter plugin resource limits are unavailable on this platform".to_string(),
    ))
}

fn wait_for_plugin(child: &mut Child, timeout: Duration) -> Result<std::process::ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(status);
        }
        if plugin_resident_bytes(child.id()).is_some_and(|bytes| bytes > PLUGIN_MEMORY_LIMIT_BYTES)
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::InvalidInput(format!(
                "adapter plugin exceeded its {} byte resident-memory limit",
                PLUGIN_MEMORY_LIMIT_BYTES
            )));
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(Error::InvalidInput(format!(
                "adapter plugin exceeded its {} ms deadline",
                timeout.as_millis()
            )));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "macos")]
fn plugin_resident_bytes(pid: u32) -> Option<u64> {
    // SAFETY: the structure is plain data and proc_pidinfo receives its exact
    // size and a valid writable pointer.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = std::mem::size_of::<libc::proc_taskinfo>();
    let read = unsafe {
        libc::proc_pidinfo(
            pid as libc::c_int,
            libc::PROC_PIDTASKINFO,
            0,
            (&mut info as *mut libc::proc_taskinfo).cast(),
            size as libc::c_int,
        )
    };
    (read == size as libc::c_int).then_some(info.pti_resident_size)
}

#[cfg(target_os = "linux")]
fn plugin_resident_bytes(pid: u32) -> Option<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let kibibytes = status.lines().find_map(|line| {
        line.strip_prefix("VmRSS:")?
            .split_whitespace()
            .next()?
            .parse::<u64>()
            .ok()
    })?;
    kibibytes.checked_mul(1024)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn plugin_resident_bytes(_pid: u32) -> Option<u64> {
    None
}

fn sandboxed_plugin_launcher(
    root: &Path,
    home: &Path,
    temporary: &Path,
    executable: &Path,
) -> Result<(PathBuf, Vec<OsString>)> {
    #[cfg(target_os = "macos")]
    {
        let launcher = PathBuf::from("/usr/bin/sandbox-exec");
        if !launcher.is_file() {
            return Err(Error::InvalidInput(
                "adapter plugins require `/usr/bin/sandbox-exec` on macOS".to_string(),
            ));
        }
        let profile = format!(
            "(version 1)\n\
             (deny default)\n\
             (import \"system.sb\")\n\
             (deny mach-lookup)\n\
             (deny mach-register)\n\
             (deny process-fork)\n\
             (deny process-exec)\n\
             (allow process-exec (literal \"{}\"))\n\
             (deny file-write*)\n\
             (allow file-write* (subpath \"{}\") (subpath \"{}\"))\n\
             (allow file-read* (subpath \"{}\") (literal \"{}\") (subpath \"/bin\") (subpath \"/usr\") (subpath \"/System\") (subpath \"/Library\") (subpath \"/opt/homebrew\") (subpath \"/nix/store\"))\n\
             (deny network*)",
            super::workspace_environment::sandbox_profile_escape(executable),
            super::workspace_environment::sandbox_profile_escape(home),
            super::workspace_environment::sandbox_profile_escape(temporary),
            super::workspace_environment::sandbox_profile_escape(root),
            super::workspace_environment::sandbox_profile_escape(executable),
        );
        Ok((
            launcher,
            vec![
                OsString::from("-p"),
                OsString::from(profile),
                executable.as_os_str().to_owned(),
            ],
        ))
    }
    #[cfg(all(any(target_os = "linux", target_os = "windows"), not(test)))]
    {
        let launcher = std::env::current_exe()?;
        Ok((
            launcher,
            vec![
                OsString::from("__environment-sandbox"),
                OsString::from("--root"),
                root.as_os_str().to_owned(),
                OsString::from("--home"),
                home.as_os_str().to_owned(),
                OsString::from("--tmp"),
                temporary.as_os_str().to_owned(),
                OsString::from("--program"),
                executable.as_os_str().to_owned(),
                OsString::from("--"),
            ],
        ))
    }
    #[cfg(any(
        all(any(target_os = "linux", target_os = "windows"), test),
        not(any(target_os = "macos", target_os = "linux", target_os = "windows"))
    ))]
    {
        let _ = (root, home, temporary, executable);
        Err(Error::InvalidInput(format!(
            "adapter plugin sandboxing is unavailable on {}; Trail refuses to execute plugin code without native enforcement",
            std::env::consts::OS
        )))
    }
}

fn plugin_request_id(
    plugin: &InstalledEnvironmentPlugin,
    source_root: &ObjectId,
    component_root: &str,
    method: &str,
    component_id: Option<&str>,
) -> String {
    sha256_hex(
        format!(
            "{}\0{}\0{}\0{}\0{}",
            plugin.distribution_digest,
            source_root.0,
            component_root,
            method,
            component_id.unwrap_or_default()
        )
        .as_bytes(),
    )
}

fn validate_discovered_plugin_component(
    plugin: &InstalledEnvironmentPlugin,
    component: &DiscoveredComponent,
) -> Result<()> {
    if component.kind != plugin.manifest.adapter.kind {
        return Err(Error::InvalidInput(format!(
            "adapter `{}` discovered kind `{}` but its package declares `{}`",
            plugin.manifest.adapter.canonical_identity,
            component.kind,
            plugin.manifest.adapter.kind
        )));
    }
    validate_plugin_component_id(&component.component_id)
}

fn validate_plugin_component_id(component_id: &str) -> Result<()> {
    if component_id.is_empty()
        || component_id.len() > 256
        || contains_sensitive_text(component_id)
        || !component_id.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-' | ':' | '/')
        })
    {
        return Err(Error::InvalidInput(format!(
            "plugin component ID `{component_id}` is invalid or sensitive"
        )));
    }
    Ok(())
}

pub(super) fn environment_plugin_supports_current_host(
    plugin: &InstalledEnvironmentPlugin,
) -> bool {
    plugin
        .manifest
        .adapter
        .supported_operating_systems
        .iter()
        .any(|value| value == std::env::consts::OS)
        && plugin
            .manifest
            .adapter
            .supported_architectures
            .iter()
            .any(|value| value == std::env::consts::ARCH)
}

pub(super) fn selected_environment_plugin_protocol(
    plugin: &InstalledEnvironmentPlugin,
) -> Result<&'static str> {
    if plugin
        .manifest
        .adapter
        .protocols
        .iter()
        .any(|protocol| protocol == PROTOCOL_V2)
    {
        Ok(PROTOCOL_V2)
    } else if plugin
        .manifest
        .adapter
        .protocols
        .iter()
        .any(|protocol| protocol == PROTOCOL_V1)
    {
        Ok(PROTOCOL_V1)
    } else {
        Err(Error::InvalidInput(format!(
            "adapter `{}` shares no supported protocol with this Trail host",
            plugin.manifest.adapter.canonical_identity
        )))
    }
}

fn ensure_environment_plugin_supports_current_host(
    plugin: &InstalledEnvironmentPlugin,
) -> Result<()> {
    if environment_plugin_supports_current_host(plugin) {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "adapter `{}` does not support {}/{}",
            plugin.manifest.adapter.canonical_identity,
            std::env::consts::OS,
            std::env::consts::ARCH
        )))
    }
}

fn normalize_plugin_component_root(component_root: &str) -> Result<String> {
    if component_root.trim_matches('/').is_empty() || component_root == "." {
        Ok(String::new())
    } else {
        normalize_relative_path(component_root)
    }
}

fn normalize_plugin_path_allow_root(path: &str) -> Result<String> {
    if path.trim_matches('/').is_empty() || path == "." {
        Ok(String::new())
    } else {
        normalize_relative_path(path)
    }
}

fn join_plugin_path(root: &str, child: &str) -> String {
    if root.is_empty() {
        child.to_string()
    } else if child.is_empty() {
        root.to_string()
    } else {
        format!("{root}/{child}")
    }
}

fn plugin_paths_overlap(left: &str, right: &str) -> bool {
    left == right
        || left.starts_with(&format!("{right}/"))
        || right.starts_with(&format!("{left}/"))
}

fn validate_plugin_output_name(name: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 128
        || contains_sensitive_text(name)
        || !name.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(Error::InvalidInput(format!(
            "plugin output name `{name}` is invalid or sensitive"
        )));
    }
    Ok(())
}

fn read_adapter_package_signature(
    package_directory: &Path,
) -> Result<Option<AdapterPackageSignature>> {
    let path = package_directory.join(PLUGIN_PACKAGE_SIGNATURE);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::Io(error)),
    };
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_PUBLISHER_DOCUMENT_BYTES
    {
        return Err(Error::InvalidInput(format!(
            "adapter signature `{}` must be a bounded regular file",
            path.display()
        )));
    }
    let signature = toml::from_str(&fs::read_to_string(&path)?).map_err(|error| {
        Error::InvalidInput(format!(
            "cannot parse adapter signature `{}`: {error}",
            path.display()
        ))
    })?;
    Ok(Some(signature))
}

fn prepare_environment_plugin_package(
    package_directory: impl AsRef<Path>,
) -> Result<PreparedEnvironmentPluginPackage> {
    let package_directory = fs::canonicalize(package_directory)?;
    if !package_directory.is_dir() {
        return Err(Error::InvalidInput(format!(
            "adapter package `{}` is not a directory",
            package_directory.display()
        )));
    }
    let manifest_path = package_directory.join(PLUGIN_PACKAGE_MANIFEST);
    let metadata = fs::symlink_metadata(&manifest_path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(Error::InvalidInput(format!(
            "adapter package manifest `{}` must be a regular file",
            manifest_path.display()
        )));
    }
    if metadata.len() > MAX_PACKAGE_MANIFEST_BYTES {
        return Err(Error::InvalidInput(format!(
            "adapter package manifest exceeds {MAX_PACKAGE_MANIFEST_BYTES} bytes"
        )));
    }
    let manifest_text = fs::read_to_string(&manifest_path)?;
    if contains_sensitive_text(&manifest_text) {
        return Err(Error::InvalidInput(
            "adapter package manifest appears to contain a secret".to_string(),
        ));
    }
    let mut package: AdapterPackageManifest = toml::from_str(&manifest_text).map_err(|error| {
        Error::InvalidInput(format!(
            "cannot parse adapter package manifest `{}`: {error}",
            manifest_path.display()
        ))
    })?;
    canonicalize_and_validate_package(&mut package)?;

    let executable_source = package_directory.join(&package.executable.path);
    let executable_metadata = fs::symlink_metadata(&executable_source)?;
    if !executable_metadata.is_file() || executable_metadata.file_type().is_symlink() {
        return Err(Error::InvalidInput(format!(
            "adapter executable `{}` must be a regular file",
            executable_source.display()
        )));
    }
    if executable_metadata.len() > MAX_PLUGIN_EXECUTABLE_BYTES {
        return Err(Error::InvalidInput(format!(
            "adapter executable exceeds {MAX_PLUGIN_EXECUTABLE_BYTES} bytes"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if executable_metadata.permissions().mode() & 0o111 == 0 {
            return Err(Error::InvalidInput(format!(
                "adapter executable `{}` is not executable",
                executable_source.display()
            )));
        }
    }
    let executable_bytes = fs::read(&executable_source)?;
    let executable_digest = format!("sha256:{}", sha256_hex(&executable_bytes));
    if package.executable.sha256 != executable_digest {
        return Err(Error::InvalidInput(format!(
            "adapter executable digest mismatch: manifest declares `{}`, actual is `{executable_digest}`",
            package.executable.sha256
        )));
    }
    let (payload_material, payload_digest) = adapter_package_payload(&package, &executable_bytes)?;
    let signature = read_adapter_package_signature(&package_directory)?;
    if let Some(signature) = &signature {
        validate_adapter_package_signature(signature)?;
        if signature.payload_digest != payload_digest {
            return Err(Error::InvalidInput(format!(
                "adapter signature authenticates `{}` but package payload is `{payload_digest}`",
                signature.payload_digest
            )));
        }
    }
    Ok(PreparedEnvironmentPluginPackage {
        package,
        executable_bytes,
        executable_permissions: executable_metadata.permissions(),
        executable_digest,
        payload_material,
        payload_digest,
        signature,
    })
}

fn adapter_package_payload(
    package: &AdapterPackageManifest,
    executable_bytes: &[u8],
) -> Result<(Vec<u8>, String)> {
    let canonical_package = cbor(package)?;
    let mut material = Vec::with_capacity(
        canonical_package.len() + executable_bytes.len() + std::mem::size_of::<u64>(),
    );
    material.extend_from_slice(&(canonical_package.len() as u64).to_be_bytes());
    material.extend_from_slice(&canonical_package);
    material.extend_from_slice(executable_bytes);
    let digest = format!("sha256:{}", sha256_hex(&material));
    Ok((material, digest))
}

fn adapter_package_distribution_material(
    payload_material: &[u8],
    signature: Option<&AdapterPackageSignature>,
) -> Result<Vec<u8>> {
    let Some(signature) = signature else {
        // Preserve the v1 unsigned package identity for existing stores.
        return Ok(payload_material.to_vec());
    };
    let signature_bytes = cbor(signature)?;
    let mut material = Vec::with_capacity(
        64 + payload_material.len() + signature_bytes.len() + 2 * std::mem::size_of::<u64>(),
    );
    material.extend_from_slice(b"trail.environment-adapter-distribution/v1\0");
    material.extend_from_slice(&(payload_material.len() as u64).to_be_bytes());
    material.extend_from_slice(payload_material);
    material.extend_from_slice(&(signature_bytes.len() as u64).to_be_bytes());
    material.extend_from_slice(&signature_bytes);
    Ok(material)
}

fn adapter_package_signature_message(payload_digest: &str) -> Vec<u8> {
    let mut message =
        Vec::with_capacity(PACKAGE_SIGNATURE_SCHEMA_V1.len() + payload_digest.len() + 1);
    message.extend_from_slice(PACKAGE_SIGNATURE_SCHEMA_V1.as_bytes());
    message.push(0);
    message.extend_from_slice(payload_digest.as_bytes());
    message
}

fn validate_publisher_name(publisher: &str) -> Result<()> {
    if publisher.is_empty()
        || publisher.len() > 128
        || contains_sensitive_text(publisher)
        || !publisher.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '-')
        })
    {
        return Err(Error::InvalidInput(format!(
            "adapter publisher `{publisher}` is invalid or sensitive"
        )));
    }
    Ok(())
}

fn validate_publisher_key_id(key_id: &str) -> Result<()> {
    let digest = key_id
        .strip_prefix("sha256:")
        .ok_or_else(|| Error::InvalidInput("publisher key ID must use sha256:<hex>".to_string()))?;
    decode_fixed_hex::<32>(digest, "publisher key ID").map(|_| ())
}

fn validate_publisher_key_document(
    document: &AdapterPublisherKey,
) -> Result<(String, String, String)> {
    if document.schema != TRUSTED_PUBLISHER_KEY_SCHEMA_V1 {
        return Err(Error::InvalidInput(format!(
            "unsupported publisher key schema `{}`; expected `{TRUSTED_PUBLISHER_KEY_SCHEMA_V1}`",
            document.schema
        )));
    }
    validate_publisher_name(&document.publisher)?;
    let public_key = decode_fixed_hex::<32>(&document.public_key, "publisher public key")?;
    VerifyingKey::from_bytes(&public_key).map_err(|error| {
        Error::InvalidInput(format!("publisher public key is invalid: {error}"))
    })?;
    let public_key_hex = hex::encode(public_key);
    let key_id = format!("sha256:{}", sha256_hex(&public_key));
    Ok((document.publisher.clone(), public_key_hex, key_id))
}

fn validate_adapter_package_signature(signature: &AdapterPackageSignature) -> Result<()> {
    if signature.schema != PACKAGE_SIGNATURE_SCHEMA_V1 {
        return Err(Error::InvalidInput(format!(
            "unsupported adapter signature schema `{}`; expected `{PACKAGE_SIGNATURE_SCHEMA_V1}`",
            signature.schema
        )));
    }
    validate_publisher_name(&signature.publisher)?;
    validate_publisher_key_id(&signature.key_id)?;
    validate_sha256_digest(&signature.payload_digest, "adapter payload digest")?;
    decode_fixed_hex::<64>(&signature.signature, "adapter signature").map(|_| ())
}

fn validate_publisher_trust_record(record: &PublisherTrustRecord, directory: &OsStr) -> Result<()> {
    if record.schema != TRUST_RECORD_SCHEMA || !matches!(record.action.as_str(), "trust" | "remove")
    {
        return Err(Error::Corrupt(
            "publisher trust record has an unsupported schema or action".to_string(),
        ));
    }
    validate_publisher_key_id(&record.key_id)
        .map_err(|error| Error::Corrupt(format!("invalid publisher trust key ID: {error}")))?;
    if directory.to_string_lossy() != record.key_id.trim_start_matches("sha256:") {
        return Err(Error::Corrupt(
            "publisher trust record is stored under the wrong key directory".to_string(),
        ));
    }
    if record.action == "trust" {
        validate_publisher_name(&record.publisher)
            .map_err(|error| Error::Corrupt(format!("invalid trusted publisher: {error}")))?;
        let public_key = decode_fixed_hex::<32>(&record.public_key, "publisher public key")
            .map_err(|error| Error::Corrupt(error.to_string()))?;
        let expected_key_id = format!("sha256:{}", sha256_hex(&public_key));
        if expected_key_id != record.key_id {
            return Err(Error::Corrupt(
                "publisher trust key ID does not match its public key".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_sha256_digest(value: &str, label: &str) -> Result<()> {
    let digest = value
        .strip_prefix("sha256:")
        .ok_or_else(|| Error::InvalidInput(format!("{label} must use sha256:<hex>")))?;
    decode_fixed_hex::<32>(digest, label).map(|_| ())
}

fn decode_fixed_hex<const N: usize>(value: &str, label: &str) -> Result<[u8; N]> {
    if value.len() != N * 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::InvalidInput(format!(
            "{label} must be {} lowercase hexadecimal characters",
            N * 2
        )));
    }
    let bytes = hex::decode(value)
        .map_err(|error| Error::InvalidInput(format!("cannot decode {label}: {error}")))?;
    bytes
        .try_into()
        .map_err(|_| Error::InvalidInput(format!("{label} has the wrong decoded byte length")))
}

fn canonicalize_and_validate_package(package: &mut AdapterPackageManifest) -> Result<()> {
    if package.schema != PACKAGE_SCHEMA_V1 {
        return Err(Error::InvalidInput(format!(
            "unsupported adapter package schema `{}`; expected `{PACKAGE_SCHEMA_V1}`",
            package.schema
        )));
    }
    let (_, _, major) = validate_plugin_identity(&package.adapter.canonical_identity)?;
    if major != 1 {
        return Err(Error::InvalidInput(format!(
            "adapter `{}` uses unsupported contract major {major}; this host supports major 1",
            package.adapter.canonical_identity
        )));
    }
    if package.adapter.implementation_version.trim().is_empty()
        || package.adapter.implementation_version.len() > 128
        || package.adapter.description.trim().is_empty()
        || package.adapter.description.len() > 4096
    {
        return Err(Error::InvalidInput(
            "adapter implementation version and description must be non-empty and bounded"
                .to_string(),
        ));
    }
    if package.adapter.stability != "experimental" {
        return Err(Error::InvalidInput(
            "unsigned local adapter packages must declare stability = \"experimental\"".to_string(),
        ));
    }
    if !matches!(
        package.adapter.kind.as_str(),
        "dependency" | "compiler-results" | "generated"
    ) {
        return Err(Error::InvalidInput(format!(
            "adapter `{}` declares unsupported kind `{}`",
            package.adapter.canonical_identity, package.adapter.kind
        )));
    }
    if package.adapter.layer_adapter_name.is_empty()
        || package.adapter.layer_adapter_name.len() > 64
        || !package.adapter.layer_adapter_name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
    {
        return Err(Error::InvalidInput(
            "adapter layer storage name must use 1-64 lowercase ASCII letters, digits, or hyphens"
                .to_string(),
        ));
    }
    sort_unique_bounded(&mut package.adapter.selectors, 32, "adapter selectors")?;
    if !package
        .adapter
        .selectors
        .contains(&package.adapter.canonical_identity)
    {
        return Err(Error::InvalidInput(
            "adapter selectors must contain canonical_identity".to_string(),
        ));
    }
    sort_unique_bounded(
        &mut package.adapter.discovery_markers,
        32,
        "adapter discovery markers",
    )?;
    sort_unique_bounded(&mut package.adapter.protocols, 4, "adapter protocols")?;
    if package.adapter.protocols.is_empty()
        || package
            .adapter
            .protocols
            .iter()
            .any(|protocol| !matches!(protocol.as_str(), PROTOCOL_V1 | PROTOCOL_V2))
    {
        return Err(Error::InvalidInput(format!(
            "adapter `{}` must declare one or more host-supported protocols",
            package.adapter.canonical_identity
        )));
    }
    for marker in &package.adapter.discovery_markers {
        if marker == "."
            || marker == ".."
            || marker.contains('/')
            || marker.contains('\\')
            || marker.contains('\0')
            || contains_sensitive_text(marker)
        {
            return Err(Error::InvalidInput(format!(
                "adapter discovery marker `{marker}` is not a safe file name"
            )));
        }
    }
    sort_unique_bounded(
        &mut package.adapter.supported_operating_systems,
        8,
        "adapter supported operating systems",
    )?;
    if package
        .adapter
        .supported_operating_systems
        .iter()
        .any(|value| !matches!(value.as_str(), "linux" | "macos" | "windows"))
    {
        return Err(Error::InvalidInput(
            "adapter supported operating systems contain an unknown value".to_string(),
        ));
    }
    sort_unique_bounded(
        &mut package.adapter.supported_architectures,
        8,
        "adapter supported architectures",
    )?;
    if package
        .adapter
        .supported_architectures
        .iter()
        .any(|value| !matches!(value.as_str(), "aarch64" | "x86_64"))
    {
        return Err(Error::InvalidInput(
            "adapter supported architectures contain an unknown value".to_string(),
        ));
    }
    sort_unique_bounded(
        &mut package.permissions.read_patterns,
        128,
        "adapter read patterns",
    )?;
    if package.permissions.read_patterns.is_empty() {
        return Err(Error::InvalidInput(
            "adapter must declare at least one repository read pattern".to_string(),
        ));
    }
    let patterns = compile_plugin_patterns(&package.permissions)?;
    for marker in &package.adapter.discovery_markers {
        if !patterns.is_match(marker) {
            return Err(Error::InvalidInput(format!(
                "adapter discovery marker `{marker}` is not covered by a declared read pattern"
            )));
        }
    }
    if package.permissions.max_input_files == 0
        || package.permissions.max_input_files > HARD_MAX_INPUT_FILES
        || package.permissions.max_input_bytes == 0
        || package.permissions.max_input_bytes > HARD_MAX_INPUT_BYTES
        || package.permissions.timeout_ms == 0
        || package.permissions.timeout_ms > HARD_MAX_TIMEOUT_MS
        || package.permissions.max_response_bytes == 0
        || package.permissions.max_response_bytes > HARD_MAX_RESPONSE_BYTES
    {
        return Err(Error::InvalidInput(
            "adapter resource limits are zero or exceed Trail's hard limits".to_string(),
        ));
    }
    let executable_path = Path::new(&package.executable.path);
    if executable_path.components().count() != 1
        || !matches!(
            executable_path.components().next(),
            Some(std::path::Component::Normal(_))
        )
    {
        return Err(Error::InvalidInput(
            "adapter executable path must be one relative file name".to_string(),
        ));
    }
    let declared_digest = package
        .executable
        .sha256
        .strip_prefix("sha256:")
        .unwrap_or(&package.executable.sha256);
    if declared_digest.len() != 64
        || !declared_digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(Error::InvalidInput(
            "adapter executable sha256 must be 64 lowercase hexadecimal characters".to_string(),
        ));
    }
    package.executable.sha256 = format!("sha256:{declared_digest}");
    Ok(())
}

pub(super) fn validate_plugin_identity(identity: &str) -> Result<(String, String, u32)> {
    let (qualified, major) = identity.rsplit_once('@').ok_or_else(|| {
        Error::InvalidInput(format!(
            "adapter identity `{identity}` must be namespace/name@major"
        ))
    })?;
    let (namespace, name) = qualified.split_once('/').ok_or_else(|| {
        Error::InvalidInput(format!(
            "adapter identity `{identity}` must be namespace/name@major"
        ))
    })?;
    let valid_segment = |segment: &str| {
        !segment.is_empty()
            && segment.len() <= 64
            && segment.chars().all(|character| {
                character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || character == '-'
                    || character == '_'
            })
    };
    if !valid_segment(namespace) || !valid_segment(name) {
        return Err(Error::InvalidInput(format!(
            "adapter identity `{identity}` contains an invalid namespace or name"
        )));
    }
    let major = major.parse::<u32>().map_err(|_| {
        Error::InvalidInput(format!(
            "adapter identity `{identity}` has an invalid contract major"
        ))
    })?;
    Ok((namespace.to_string(), name.to_string(), major))
}

fn sort_unique_bounded(values: &mut Vec<String>, maximum: usize, label: &str) -> Result<()> {
    if values.is_empty() || values.len() > maximum {
        return Err(Error::InvalidInput(format!(
            "{label} must contain between 1 and {maximum} entries"
        )));
    }
    if values
        .iter()
        .any(|value| value.trim().is_empty() || value.len() > 4096 || value.contains('\0'))
    {
        return Err(Error::InvalidInput(format!(
            "{label} contains an empty, oversized, or NUL-bearing value"
        )));
    }
    values.sort();
    values.dedup();
    Ok(())
}

fn compile_plugin_patterns(permissions: &AdapterPermissions) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pattern in &permissions.read_patterns {
        if pattern.starts_with('/')
            || pattern.starts_with("../")
            || pattern.contains("/../")
            || pattern.contains('\\')
            || contains_sensitive_text(pattern)
        {
            return Err(Error::InvalidInput(format!(
                "adapter read pattern `{pattern}` is unsafe"
            )));
        }
        let glob = GlobBuilder::new(pattern)
            .literal_separator(true)
            .backslash_escape(false)
            .build()
            .map_err(|error| {
                Error::InvalidInput(format!(
                    "adapter read pattern `{pattern}` is invalid: {error}"
                ))
            })?;
        builder.add(glob);
    }
    builder.build().map_err(|error| {
        Error::InvalidInput(format!("cannot compile adapter read patterns: {error}"))
    })
}

fn validate_registry_record(record: &PluginRegistryRecord, directory_name: &OsStr) -> Result<()> {
    if record.schema != REGISTRY_RECORD_SCHEMA
        || !matches!(record.action.as_str(), "install" | "remove")
    {
        return Err(Error::Corrupt(
            "adapter registry record has an unsupported schema or action".to_string(),
        ));
    }
    validate_plugin_identity(&record.canonical_identity).map_err(|error| {
        Error::Corrupt(format!("adapter registry identity is invalid: {error}"))
    })?;
    let expected_directory = sha256_hex(record.canonical_identity.as_bytes());
    if directory_name != OsStr::new(&expected_directory) {
        return Err(Error::Corrupt(format!(
            "adapter registry identity directory does not match `{}`",
            record.canonical_identity
        )));
    }
    if record.action == "install"
        && !record
            .distribution_digest
            .as_deref()
            .is_some_and(valid_sha256_digest)
    {
        return Err(Error::Corrupt(
            "adapter install record has an invalid distribution digest".to_string(),
        ));
    }
    if record.action == "remove" && record.distribution_digest.is_some() {
        return Err(Error::Corrupt(
            "adapter removal record unexpectedly names a distribution".to_string(),
        ));
    }
    Ok(())
}

fn valid_sha256_digest(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|digest| {
        digest.len() == 64
            && digest
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

fn validate_plugin_catalog(plugins: &[InstalledEnvironmentPlugin]) -> Result<()> {
    let mut identities = BTreeSet::new();
    let mut selectors = BTreeMap::<&str, &str>::new();
    for plugin in plugins {
        let identity = plugin.manifest.adapter.canonical_identity.as_str();
        if !identities.insert(identity) {
            return Err(Error::Corrupt(format!(
                "adapter registry contains duplicate identity `{identity}`"
            )));
        }
        for selector in &plugin.manifest.adapter.selectors {
            if let Some(other) = selectors.insert(selector, identity) {
                return Err(Error::Corrupt(format!(
                    "adapter selector `{selector}` is claimed by both `{other}` and `{identity}`"
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey};

    #[test]
    fn protocol_v2_typed_dependencies_normalize_to_host_edge_semantics() {
        let plan = AdapterPlanV2::builder("application", "generated")
            .build_requires("compiler")
            .runtime_requires("database")
            .binds_after("network")
            .invalidates_with("configuration")
            .mounted_command(AdapterCommand::new("initializer", ["prepare"]))
            .output(AdapterOutput::writable_private("state", "state", "state"))
            .stale_reason("typed dependency changed")
            .build()
            .unwrap();
        let normalized = ProposedPluginPlan::from_v2(plan).unwrap();
        assert_eq!(
            normalized
                .dependencies
                .iter()
                .map(|dependency| (
                    dependency.component_id.as_str(),
                    dependency.edge_type.as_str()
                ))
                .collect::<Vec<_>>(),
            [
                ("compiler", "build_requires"),
                ("configuration", "invalidates_with"),
                ("database", "runtime_requires"),
                ("network", "binds_after")
            ]
        );
    }

    #[test]
    fn protocol_v2_runtime_resources_survive_host_protocol_normalization() {
        let digest = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let plan = AdapterPlanV2::builder("services", "external")
            .external_artifact(AdapterExternalArtifact::pinned_oci_image(
                "database-image",
                format!("example.invalid/postgres@{digest}"),
                "linux/amd64",
            ))
            .runtime_resource(
                AdapterRuntimeResource::oci_container("database", "database-image", 5432)
                    .health_timeout_ms(45_000)
                    .volume_target("/var/lib/postgresql/data"),
            )
            .stale_reason("service image or runtime contract changed")
            .build()
            .unwrap();

        let normalized = ProposedPluginPlan::from_v2(plan).unwrap();
        assert_eq!(normalized.external_artifacts.len(), 1);
        assert_eq!(normalized.runtime_resources.len(), 1);
        assert_eq!(normalized.runtime_resources[0].name, "database");
        assert_eq!(normalized.runtime_resources[0].container_port, 5432);
        assert_eq!(
            normalized.runtime_resources[0].volume_target.as_deref(),
            Some("/var/lib/postgresql/data")
        );
    }

    #[test]
    fn protocol_v2_plugin_caches_are_host_scoped_and_fail_closed() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let package = tempfile::tempdir().unwrap();
        write_test_package(package.path(), "example/cache-test@1");
        db.install_environment_adapter_plugin(package.path())
            .unwrap();
        let mut plugins = db.installed_environment_plugins().unwrap();
        let plugin = plugins.remove(0);

        let cache =
            AdapterCache::host_exclusive("package-store", AdapterCacheProtocol::ContentStore)
                .compatibility_dimension("tool", "fixture@1")
                .environment_variable("FIXTURE_CACHE", "store");
        let (normalized, environment) = db
            .normalize_environment_plugin_caches(&plugin, PROTOCOL_V2, &[cache], true)
            .unwrap();
        assert_eq!(normalized.len(), 1);
        assert_eq!(
            normalized[0].access,
            WorkspaceEnvironmentCacheAccess::HostExclusive
        );
        assert_eq!(
            normalized[0].compatibility["trail.adapter_distribution"],
            plugin.distribution_digest
        );
        assert_eq!(
            environment["FIXTURE_CACHE"],
            normalized[0]
                .storage_path
                .join("store")
                .to_string_lossy()
                .into_owned()
        );
        assert!(!normalized[0].storage_path.exists());

        let concurrent =
            AdapterCache::tool_concurrent("unsafe-store", AdapterCacheProtocol::LockedIndex)
                .environment_variable("FIXTURE_CACHE", ".");
        assert!(db
            .normalize_environment_plugin_caches(&plugin, PROTOCOL_V2, &[concurrent], true)
            .unwrap_err()
            .to_string()
            .contains("without independent cache certification"));

        let sensitive =
            AdapterCache::host_exclusive("secret-store", AdapterCacheProtocol::ContentStore)
                .compatibility_dimension("access_token", "do-not-persist")
                .environment_variable("FIXTURE_CACHE", ".");
        assert!(db
            .normalize_environment_plugin_caches(&plugin, PROTOCOL_V2, &[sensitive], true)
            .unwrap_err()
            .to_string()
            .contains("secret-like data"));
    }

    fn write_test_package(root: &Path, identity: &str) {
        let executable = root.join("adapter-test");
        fs::write(&executable, b"test adapter executable\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        }
        let digest = sha256_hex(&fs::read(&executable).unwrap());
        fs::write(
            root.join(PLUGIN_PACKAGE_MANIFEST),
            format!(
                r#"schema = "trail.environment-adapter-package/v1"

[adapter]
canonical_identity = "{identity}"
implementation_version = "1.0.0"
selectors = ["{identity}", "test-plugin"]
kind = "generated"
layer_adapter_name = "test-plugin"
discovery_markers = ["test.plugin"]
stability = "experimental"
description = "Test adapter"

[executable]
path = "adapter-test"
sha256 = "{digest}"

[permissions]
read_patterns = ["test.plugin", "config/*.toml"]
max_input_files = 16
max_input_bytes = 1048576
timeout_ms = 1000
max_response_bytes = 1048576
"#
            ),
        )
        .unwrap();
    }

    fn sign_test_package(root: &Path, signing_key: &SigningKey, publisher: &str) -> PathBuf {
        let mut package: AdapterPackageManifest =
            toml::from_str(&fs::read_to_string(root.join(PLUGIN_PACKAGE_MANIFEST)).unwrap())
                .unwrap();
        canonicalize_and_validate_package(&mut package).unwrap();
        let executable = fs::read(root.join(&package.executable.path)).unwrap();
        let (_, payload_digest) = adapter_package_payload(&package, &executable).unwrap();
        let verifying_key = signing_key.verifying_key();
        let public_key = verifying_key.to_bytes();
        let key_id = format!("sha256:{}", sha256_hex(&public_key));
        let signature = signing_key.sign(&adapter_package_signature_message(&payload_digest));
        let signature_document = AdapterPackageSignature {
            schema: PACKAGE_SIGNATURE_SCHEMA_V1.to_string(),
            publisher: publisher.to_string(),
            key_id: key_id.clone(),
            payload_digest,
            signature: hex::encode(signature.to_bytes()),
        };
        fs::write(
            root.join(PLUGIN_PACKAGE_SIGNATURE),
            toml::to_string(&signature_document).unwrap(),
        )
        .unwrap();
        let key_path = root.join("publisher-key.toml");
        fs::write(
            &key_path,
            toml::to_string(&AdapterPublisherKey {
                schema: TRUSTED_PUBLISHER_KEY_SCHEMA_V1.to_string(),
                publisher: publisher.to_string(),
                public_key: hex::encode(public_key),
            })
            .unwrap(),
        )
        .unwrap();
        key_path
    }

    #[test]
    fn plugin_install_is_content_addressed_catalogued_and_tombstoned() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let package = tempfile::tempdir().unwrap();
        write_test_package(package.path(), "example/test@1");

        let inspected = db
            .inspect_environment_adapter_plugin_package(package.path())
            .unwrap();
        assert_eq!(inspected.canonical_identity, "example/test@1");
        assert!(!inspected.signature_present);

        let installed = db
            .install_environment_adapter_plugin(package.path())
            .unwrap();
        assert_eq!(installed.canonical_identity, "example/test@1");
        assert_eq!(installed.distribution_digest, inspected.distribution_digest);
        assert!(installed.distribution_digest.starts_with("sha256:"));
        assert_eq!(installed.trust, "local_unsigned");
        assert_eq!(installed.certification_tier, "local-experimental");
        assert!(installed.publisher.is_none());
        assert!(Path::new(&installed.package_path).is_dir());
        let catalog = db.workspace_environment_adapters().unwrap();
        let entry = catalog
            .adapters
            .iter()
            .find(|entry| entry.canonical_identity == "example/test@1")
            .unwrap();
        assert_eq!(entry.source, "plugin");
        assert_eq!(entry.trust, "local_unsigned");
        assert_eq!(entry.certification_tier, "local-experimental");
        assert_eq!(
            entry.identity.distribution_digest.as_deref(),
            Some(installed.distribution_digest.as_str())
        );

        let repeated = db
            .install_environment_adapter_plugin(package.path())
            .unwrap();
        assert_eq!(
            repeated.replaced_distribution_digest.as_deref(),
            Some(installed.distribution_digest.as_str())
        );
        let removed = db
            .remove_environment_adapter_plugin("example/test@1")
            .unwrap();
        assert_eq!(
            removed.removed_distribution_digest.as_deref(),
            Some(installed.distribution_digest.as_str())
        );
        assert!(db
            .workspace_environment_adapters()
            .unwrap()
            .adapters
            .iter()
            .all(|entry| entry.canonical_identity != "example/test@1"));
    }

    #[test]
    fn signed_plugin_requires_live_publisher_trust_and_reports_authentication() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let package = tempfile::tempdir().unwrap();
        write_test_package(package.path(), "example/signed@1");
        let signing_key = SigningKey::from_bytes(&[7_u8; 32]);
        let key_document = sign_test_package(package.path(), &signing_key, "example-publisher");

        let inspected = db
            .inspect_environment_adapter_plugin_package(package.path())
            .unwrap();
        assert!(inspected.signature_present);
        assert_eq!(inspected.publisher.as_deref(), Some("example-publisher"));

        let untrusted = db
            .install_environment_adapter_plugin(package.path())
            .unwrap_err();
        assert!(untrusted.to_string().contains("is not trusted"));

        let trusted = db
            .trust_environment_adapter_publisher_key(&key_document)
            .unwrap();
        assert_eq!(trusted.action, "trust");
        assert_eq!(trusted.publisher.as_deref(), Some("example-publisher"));
        let installed = db
            .install_environment_adapter_plugin(package.path())
            .unwrap();
        assert_eq!(installed.publisher.as_deref(), Some("example-publisher"));
        assert_eq!(
            installed.publisher_key_id.as_deref(),
            Some(trusted.key_id.as_str())
        );
        assert_eq!(installed.trust, "publisher_signed");
        assert_eq!(
            installed.certification_tier,
            "publisher-authenticated-experimental"
        );
        let entry = db
            .workspace_environment_adapters()
            .unwrap()
            .adapters
            .into_iter()
            .find(|entry| entry.canonical_identity == "example/signed@1")
            .unwrap();
        assert_eq!(entry.publisher.as_deref(), Some("example-publisher"));
        assert_eq!(entry.publisher_key_id, installed.publisher_key_id);
        assert_eq!(entry.trust, "publisher_signed");

        let trust = db.environment_adapter_publisher_trust().unwrap();
        assert_eq!(trust.keys.len(), 1);
        assert_eq!(trust.keys[0].key_id, trusted.key_id);
        db.remove_environment_adapter_publisher_key(&trusted.key_id)
            .unwrap();
        assert!(db
            .workspace_environment_adapters()
            .unwrap_err()
            .to_string()
            .contains("no longer has a valid trusted publisher"));
        assert!(db
            .environment_adapter_publisher_trust()
            .unwrap()
            .keys
            .is_empty());

        db.trust_environment_adapter_publisher_key(&key_document)
            .unwrap();
        assert!(db.workspace_environment_adapters().is_ok());
        let signature_path = package.path().join(PLUGIN_PACKAGE_SIGNATURE);
        let mut signature: AdapterPackageSignature =
            toml::from_str(&fs::read_to_string(&signature_path).unwrap()).unwrap();
        let mut signature_bytes = hex::decode(&signature.signature).unwrap();
        signature_bytes[0] ^= 1;
        signature.signature = hex::encode(signature_bytes);
        fs::write(&signature_path, toml::to_string(&signature).unwrap()).unwrap();
        let invalid = db
            .install_environment_adapter_plugin(package.path())
            .unwrap_err();
        assert!(invalid.to_string().contains("signature"));

        let removed = db
            .remove_environment_adapter_plugin("example/signed@1")
            .unwrap();
        assert!(removed.removed_distribution_digest.is_some());
    }

    #[test]
    fn plugin_catalog_fails_closed_after_executable_tampering() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let package = tempfile::tempdir().unwrap();
        write_test_package(package.path(), "example/test@1");
        let installed = db
            .install_environment_adapter_plugin(package.path())
            .unwrap();
        fs::write(
            Path::new(&installed.package_path).join("adapter-test"),
            b"tampered\n",
        )
        .unwrap();
        let error = db.workspace_environment_adapters().unwrap_err();
        assert!(error.to_string().contains("digest verification"));

        let repaired = db
            .install_environment_adapter_plugin(package.path())
            .unwrap();
        assert_eq!(repaired.canonical_identity, "example/test@1");
        assert!(db.workspace_environment_adapters().is_ok());
        fs::write(
            Path::new(&repaired.package_path).join("adapter-test"),
            b"tampered again\n",
        )
        .unwrap();
        let removed = db
            .remove_environment_adapter_plugin("example/test@1")
            .unwrap();
        assert!(removed.removed_distribution_digest.is_some());
        assert!(db.workspace_environment_adapters().is_ok());
    }
}
