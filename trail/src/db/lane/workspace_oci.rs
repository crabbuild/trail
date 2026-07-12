use serde::Deserialize;

use super::workspace_environment::{
    workspace_external_artifacts_identity, workspace_runtime_resources_identity,
    WorkspaceEnvironmentAdapter, WorkspaceEnvironmentAdapterMetadata,
    WorkspaceEnvironmentExternalArtifact, WorkspaceEnvironmentPlan,
    WorkspaceEnvironmentRuntimeResource, WorkspaceEnvironmentSandboxPolicy,
    WorkspaceEnvironmentSecretReference,
};
use super::*;

const OCI_SPEC_SCHEMA: &str = "trail.oci-images/v1";
const OCI_SPEC_FILE: &str = "trail.oci.toml";
const MAX_OCI_SPEC_BYTES: u64 = 1024 * 1024;

pub(crate) struct OciImageAdapter;

pub(crate) static OCI_IMAGE_ADAPTER: OciImageAdapter = OciImageAdapter;

static OCI_IMAGE_ADAPTER_METADATA: WorkspaceEnvironmentAdapterMetadata =
    WorkspaceEnvironmentAdapterMetadata {
        canonical_identity: "trail/oci-image@1",
        namespace: "trail",
        name: "oci-image",
        contract_major: 1,
        implementation_version: env!("CARGO_PKG_VERSION"),
        distribution_digest: "builtin:pinned-oci-image-plan-v1",
        selectors: &["trail/oci-image@1", "oci-image", "oci"],
        kind: "external",
        layer_adapter_name: "oci-image",
        discovery_markers: &[OCI_SPEC_FILE],
        supported_operating_systems: &["linux", "macos", "windows"],
        supported_architectures: &["aarch64", "x86_64"],
        stability: "experimental",
        description: "Digest-pinned OCI image identities recorded atomically with a lane environment generation",
    };

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OciImageSpecification {
    schema: String,
    #[serde(default, rename = "image")]
    images: Vec<OciImageDeclaration>,
    #[serde(default, rename = "service")]
    services: Vec<OciServiceDeclaration>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OciImageDeclaration {
    name: String,
    reference: String,
    platform: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OciServiceDeclaration {
    name: String,
    image: String,
    container_port: u16,
    #[serde(default = "default_health_timeout_ms")]
    health_timeout_ms: u64,
    #[serde(default = "default_restart_policy")]
    restart_policy: String,
    #[serde(default)]
    volume_target: Option<String>,
    #[serde(default, rename = "secret")]
    secrets: Vec<OciServiceSecretDeclaration>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct OciServiceSecretDeclaration {
    name: String,
    provider: String,
    reference: String,
    #[serde(default)]
    version: Option<String>,
    purpose: String,
    injection: String,
    target: String,
    #[serde(default)]
    environment: Option<String>,
    #[serde(default = "default_secret_required")]
    required: bool,
}

fn default_secret_required() -> bool {
    true
}

fn default_health_timeout_ms() -> u64 {
    30_000
}

fn default_restart_policy() -> String {
    "on_failure".to_string()
}

impl WorkspaceEnvironmentAdapter for OciImageAdapter {
    fn metadata(&self) -> &'static WorkspaceEnvironmentAdapterMetadata {
        &OCI_IMAGE_ADAPTER_METADATA
    }

    fn component_id(&self, component_root: &str) -> Result<String> {
        let root = normalize_component_root(component_root)?;
        Ok(if root.is_empty() {
            "oci-images".to_string()
        } else {
            format!("oci-images:{root}")
        })
    }

    fn detect(&self, db: &Trail, source_root: &ObjectId, component_root: &str) -> Result<bool> {
        let root = normalize_component_root(component_root)?;
        Ok(db
            .root_file_entry(source_root, &join_repo_path(&root, OCI_SPEC_FILE))?
            .is_some())
    }

    fn plan(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<WorkspaceEnvironmentPlan> {
        let component_root = normalize_component_root(component_root)?;
        let specification_path = join_repo_path(&component_root, OCI_SPEC_FILE);
        let entry = db
            .root_file_entry(source_root, &specification_path)?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "OCI component `{}` has no {OCI_SPEC_FILE}",
                    display_component_root(&component_root)
                ))
            })?;
        if entry.size_bytes > MAX_OCI_SPEC_BYTES {
            return Err(Error::InvalidInput(format!(
                "OCI specification `{specification_path}` is {} bytes; the maximum is {MAX_OCI_SPEC_BYTES}",
                entry.size_bytes
            )));
        }
        let manifest_digest = entry.content_hash.clone();
        let mut bytes =
            db.materialize_entries_bytes(&BTreeMap::from([(specification_path.clone(), entry)]))?;
        let text = String::from_utf8(bytes.remove(&specification_path).ok_or_else(|| {
            Error::Corrupt(format!(
                "failed to read `{specification_path}` from the pinned source root"
            ))
        })?)
        .map_err(|_| {
            Error::InvalidInput(format!(
                "OCI specification `{specification_path}` must be UTF-8"
            ))
        })?;
        let specification: OciImageSpecification = toml::from_str(&text).map_err(|error| {
            Error::InvalidInput(format!(
                "invalid OCI specification `{specification_path}`: {error}"
            ))
        })?;
        if specification.schema != OCI_SPEC_SCHEMA {
            return Err(Error::InvalidInput(format!(
                "OCI specification `{specification_path}` uses schema `{}`; expected `{OCI_SPEC_SCHEMA}`",
                specification.schema
            )));
        }
        if specification.images.is_empty() {
            return Err(Error::InvalidInput(format!(
                "OCI specification `{specification_path}` must declare at least one [[image]]"
            )));
        }
        if specification.images.len() > 32 {
            return Err(Error::InvalidInput(format!(
                "OCI specification `{specification_path}` declares more than 32 images"
            )));
        }

        let mut names = BTreeSet::new();
        let mut external_artifacts = Vec::with_capacity(specification.images.len());
        for image in specification.images {
            if !names.insert(image.name.clone()) {
                return Err(Error::InvalidInput(format!(
                    "OCI specification `{specification_path}` repeats image `{}`",
                    image.name
                )));
            }
            let digest = image
                .reference
                .rsplit_once('@')
                .map(|(_, digest)| digest.to_string())
                .ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "OCI image `{}` must use a digest-pinned reference such as registry.example/repository@sha256:<digest>",
                        image.name
                    ))
                })?;
            external_artifacts.push(WorkspaceEnvironmentExternalArtifact {
                name: image.name,
                artifact_type: "oci_image".to_string(),
                provider: "oci".to_string(),
                reference: image.reference,
                digest,
                platform: image.platform,
                cleanup_owner: "external".to_string(),
            });
        }
        external_artifacts.sort_by(|left, right| left.name.cmp(&right.name));
        let external_artifact_contract =
            workspace_external_artifacts_identity(&external_artifacts)?;
        if specification.services.len() > 32 {
            return Err(Error::InvalidInput(format!(
                "OCI specification `{specification_path}` declares more than 32 services"
            )));
        }
        let artifact_names = external_artifacts
            .iter()
            .map(|artifact| artifact.name.as_str())
            .collect::<BTreeSet<_>>();
        let mut service_names = BTreeSet::new();
        let mut runtime_resources = Vec::with_capacity(specification.services.len());
        for service in specification.services {
            if !service_names.insert(service.name.clone()) {
                return Err(Error::InvalidInput(format!(
                    "OCI specification `{specification_path}` repeats service `{}`",
                    service.name
                )));
            }
            if !artifact_names.contains(service.image.as_str()) {
                return Err(Error::InvalidInput(format!(
                    "OCI service `{}` references undeclared image `{}`",
                    service.name, service.image
                )));
            }
            runtime_resources.push(WorkspaceEnvironmentRuntimeResource {
                name: service.name,
                runtime_type: "container".to_string(),
                provider: "oci".to_string(),
                artifact_name: service.image,
                container_port: service.container_port,
                protocol: "tcp".to_string(),
                health_type: "tcp".to_string(),
                health_timeout_ms: service.health_timeout_ms,
                restart_policy: service.restart_policy,
                cleanup_owner: "trail".to_string(),
                volume_target: service.volume_target,
                secrets: service
                    .secrets
                    .into_iter()
                    .map(|secret| WorkspaceEnvironmentSecretReference {
                        name: secret.name,
                        provider: secret.provider,
                        reference: secret.reference,
                        version: secret.version,
                        purpose: secret.purpose,
                        injection: secret.injection,
                        target: secret.target,
                        environment: secret.environment,
                        required: secret.required,
                    })
                    .collect(),
            });
        }
        runtime_resources.sort_by(|left, right| left.name.cmp(&right.name));
        let runtime_resource_contract = workspace_runtime_resources_identity(&runtime_resources)?;
        let component_id = self.component_id(&component_root)?;
        let implementation_version = env!("CARGO_PKG_VERSION").to_string();
        let distribution_digest = "builtin:pinned-oci-image-plan-v1".to_string();
        let mut layer_inputs = BTreeMap::from([
            ("component_id".to_string(), component_id.clone()),
            ("component_root".to_string(), component_root.clone()),
            ("source_root".to_string(), source_root.0.clone()),
            ("manifest".to_string(), specification_path),
            ("manifest_digest".to_string(), manifest_digest),
            (
                "external_artifact_contract".to_string(),
                external_artifact_contract,
            ),
            (
                "adapter_implementation".to_string(),
                implementation_version.clone(),
            ),
            (
                "adapter_distribution_digest".to_string(),
                distribution_digest.clone(),
            ),
        ]);
        if !runtime_resources.is_empty() {
            layer_inputs.insert(
                "runtime_resource_contract".to_string(),
                runtime_resource_contract,
            );
        }

        Ok(WorkspaceEnvironmentPlan {
            component_id: component_id.clone(),
            adapter_identity: self.identity().to_string(),
            adapter_version: 1,
            implementation_version: implementation_version.clone(),
            distribution_digest: distribution_digest.clone(),
            kind: "external".to_string(),
            dependencies: Vec::new(),
            resolved_dependencies: Vec::new(),
            layer_key: WorkspaceLayerKeyV1 {
                kind: "external".to_string(),
                adapter: self.layer_adapter_name().to_string(),
                adapter_version: 1,
                inputs: layer_inputs,
                tool_versions: BTreeMap::new(),
                platform: "oci".to_string(),
                architecture: "declared-per-artifact".to_string(),
                portability_scope: "external-oci-digest-platform".to_string(),
                strategy: "pinned-oci-images-v1".to_string(),
            },
            inputs: Vec::new(),
            source_projection: None,
            pre_commands: Vec::new(),
            command: None,
            mounted_commands: Vec::new(),
            caches: Vec::new(),
            external_artifacts,
            runtime_resources,
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
            outputs: Vec::new(),
            stale_reason: "pinned OCI reference, digest, platform, or adapter policy changed"
                .to_string(),
        })
    }
}

fn normalize_component_root(component_root: &str) -> Result<String> {
    if component_root.trim_matches('/').is_empty() {
        Ok(String::new())
    } else {
        normalize_relative_path(component_root)
    }
}

fn join_repo_path(root: &str, name: &str) -> String {
    if root.is_empty() {
        name.to_string()
    } else {
        format!("{root}/{name}")
    }
}

fn display_component_root(component_root: &str) -> &str {
    if component_root.is_empty() {
        "."
    } else {
        component_root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIGEST: &str = "sha256:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn write_specification(root: &Path, reference: &str) {
        fs::write(
            root.join(OCI_SPEC_FILE),
            format!(
                "schema = \"{OCI_SPEC_SCHEMA}\"\n\n[[image]]\nname = \"web\"\nreference = \"{reference}\"\nplatform = \"linux/amd64\"\n"
            ),
        )
        .unwrap();
    }

    fn write_service_specification(root: &Path, reference: &str) {
        fs::write(
            root.join(OCI_SPEC_FILE),
            format!(
                "schema = \"{OCI_SPEC_SCHEMA}\"\n\n[[image]]\nname = \"database-image\"\nreference = \"{reference}\"\nplatform = \"linux/amd64\"\n\n[[service]]\nname = \"database\"\nimage = \"database-image\"\ncontainer_port = 5432\nhealth_timeout_ms = 45000\nrestart_policy = \"on_failure\"\nvolume_target = \"/var/lib/postgresql/data\"\n\n[[service.secret]]\nname = \"database-password\"\nprovider = \"environment_file\"\nreference = \"DATABASE_PASSWORD_FILE\"\nversion = \"rotation-7\"\npurpose = \"authenticate the database service\"\ninjection = \"file\"\ntarget = \"/run/secrets/database-password\"\nenvironment = \"POSTGRES_PASSWORD_FILE\"\nrequired = true\n"
            ),
        )
        .unwrap();
    }

    #[test]
    fn discovery_and_planning_are_pinned_and_side_effect_free() {
        let workspace = tempfile::tempdir().unwrap();
        write_specification(workspace.path(), &format!("ghcr.io/example/web@{DIGEST}"));
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let root = db.resolve_branch_ref("main").unwrap().root_id;
        let cache_root = workspace.path().join(".trail/cache/environments");
        let before = fs::read_dir(&cache_root)
            .ok()
            .map(|entries| entries.count())
            .unwrap_or_default();

        assert!(OCI_IMAGE_ADAPTER.detect(&db, &root, "").unwrap());
        let plan = OCI_IMAGE_ADAPTER.plan(&db, &root, "").unwrap();
        assert_eq!(plan.component_id, "oci-images");
        assert_eq!(plan.kind, "external");
        assert!(plan.outputs.is_empty());
        assert!(plan.command.is_none());
        assert!(plan.caches.is_empty());
        assert_eq!(plan.external_artifacts.len(), 1);
        assert_eq!(plan.external_artifacts[0].digest, DIGEST);
        assert_eq!(plan.external_artifacts[0].platform, "linux/amd64");

        let after = fs::read_dir(&cache_root)
            .ok()
            .map(|entries| entries.count())
            .unwrap_or_default();
        assert_eq!(after, before);
    }

    #[test]
    fn planning_rejects_tags_without_a_digest() {
        let workspace = tempfile::tempdir().unwrap();
        write_specification(workspace.path(), "ghcr.io/example/web:latest");
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let root = db.resolve_branch_ref("main").unwrap().root_id;
        let error = OCI_IMAGE_ADAPTER.plan(&db, &root, "").unwrap_err();
        assert!(error.to_string().contains("digest-pinned reference"));
    }

    #[test]
    fn two_lanes_activate_independent_metadata_generations_without_fake_layers() {
        let workspace = tempfile::tempdir().unwrap();
        write_specification(workspace.path(), &format!("ghcr.io/example/web@{DIGEST}"));
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        for lane in ["oci-a", "oci-b"] {
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                lane,
                Some("main"),
                if cfg!(target_os = "macos") {
                    LaneWorkdirMode::NfsCow
                } else if cfg!(target_os = "windows") {
                    LaneWorkdirMode::DokanCow
                } else {
                    LaneWorkdirMode::FuseCow
                },
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
        }

        let first = db
            .sync_workspace_environment_component("oci-a", OCI_IMAGE_ADAPTER.identity(), None, None)
            .unwrap();
        let second = db
            .sync_workspace_environment_component("oci-b", OCI_IMAGE_ADAPTER.identity(), None, None)
            .unwrap();

        assert!(first.layers.is_empty());
        assert!(second.layers.is_empty());
        assert_ne!(
            first.generation.generation_id,
            second.generation.generation_id
        );
        assert_eq!(
            first.generation.components[0].component_key,
            second.generation.components[0].component_key
        );
        assert!(first.generation.components[0].outputs.is_empty());
        assert_eq!(
            first.generation.components[0].external_artifacts,
            second.generation.components[0].external_artifacts
        );
        assert_eq!(
            first.generation.components[0].external_artifacts[0].digest,
            DIGEST
        );
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }

    #[test]
    fn service_declarations_allocate_distinct_pending_runtime_identities_per_lane() {
        let workspace = tempfile::tempdir().unwrap();
        write_service_specification(
            workspace.path(),
            &format!("ghcr.io/example/postgres@{DIGEST}"),
        );
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        for lane in ["runtime-a", "runtime-b"] {
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                lane,
                Some("main"),
                if cfg!(target_os = "macos") {
                    LaneWorkdirMode::NfsCow
                } else if cfg!(target_os = "windows") {
                    LaneWorkdirMode::DokanCow
                } else {
                    LaneWorkdirMode::FuseCow
                },
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
        }

        let plan = db
            .plan_workspace_environment_component(
                "runtime-a",
                OCI_IMAGE_ADAPTER.identity(),
                None,
                None,
            )
            .unwrap();
        assert_eq!(plan.runtime_resources.len(), 1);
        assert_eq!(plan.runtime_resources[0].name, "database");
        assert_eq!(plan.runtime_resources[0].container_port, 5432);
        assert_eq!(
            plan.runtime_resources[0].volume_target.as_deref(),
            Some("/var/lib/postgresql/data")
        );
        assert_eq!(plan.runtime_resources[0].secrets.len(), 1);
        assert_eq!(
            plan.runtime_resources[0].secrets[0].reference,
            "DATABASE_PASSWORD_FILE"
        );
        assert_eq!(
            plan.runtime_resources[0].secrets[0].environment.as_deref(),
            Some("POSTGRES_PASSWORD_FILE")
        );
        assert!(plan.outputs.is_empty());

        let first = db
            .sync_workspace_environment_component(
                "runtime-a",
                OCI_IMAGE_ADAPTER.identity(),
                None,
                None,
            )
            .unwrap();
        let second = db
            .sync_workspace_environment_component(
                "runtime-b",
                OCI_IMAGE_ADAPTER.identity(),
                None,
                None,
            )
            .unwrap();
        let first_resource = &first.generation.components[0].runtime_resources[0];
        let second_resource = &second.generation.components[0].runtime_resources[0];
        assert_eq!(first_resource.status, "pending");
        assert_eq!(first_resource.health_status, "pending");
        assert_eq!(first_resource.image_digest, DIGEST);
        assert_eq!(first_resource.secret_statuses.len(), 1);
        assert_eq!(first_resource.secret_statuses[0].status, "pending");
        assert_ne!(first_resource.allocation_id, second_resource.allocation_id);
        assert_ne!(
            first_resource.container_name,
            second_resource.container_name
        );
        assert_ne!(first_resource.network_name, second_resource.network_name);
        assert_ne!(first_resource.volume_name, second_resource.volume_name);
        assert!(first.layers.is_empty());
        assert!(second.layers.is_empty());
        assert!(db
            .lane_readiness("runtime-a")
            .unwrap()
            .blockers
            .iter()
            .any(|blocker| blocker.code == "environment_runtime_unhealthy"));

        drop(db);
        let reopened = Trail::open(workspace.path()).unwrap();
        let persisted = reopened
            .active_environment_generation("runtime-a")
            .unwrap()
            .unwrap();
        assert_eq!(
            persisted.components[0].runtime_resources[0],
            first_resource.clone()
        );
    }
}
