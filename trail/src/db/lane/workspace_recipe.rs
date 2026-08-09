use globset::{GlobBuilder, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use crate::ids::ArtifactDesiredKeyV2;

use super::workspace_environment::{
    resolve_workspace_tool_executable, validate_environment_output_contract, ResolvedWorkspaceTool,
    WorkspaceEnvironmentAdapterMetadata, WorkspaceEnvironmentCommand,
    WorkspaceEnvironmentDependency, WorkspaceEnvironmentEdgeType, WorkspaceEnvironmentInput,
    WorkspaceEnvironmentOutput, WorkspaceEnvironmentPlan, WorkspaceEnvironmentSandboxPolicy,
};
use super::*;

const RECIPE_SCHEMA_V1: &str = "trail.environment/v1";
const RECIPE_SCHEMA_V2: &str = "trail.environment/v2";
const RECIPE_ADAPTER_IDENTITY: &str = "trail/command@1";
const RECIPE_SPEC_PATHS: [&str; 2] = ["trail.environment.toml", ".trail/environment.toml"];
const MAX_RECIPE_SPEC_BYTES: u64 = 1024 * 1024;
const MAX_RECIPE_TOTAL_SPEC_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RECIPE_INCLUDE_FILES: usize = 32;
const MAX_RECIPE_INCLUDE_DEPTH: usize = 8;
const MAX_RECIPE_INPUT_DECLARATIONS: usize = 4_096;
const MAX_RECIPE_INPUT_FILES: usize = 100_000;
const MAX_RECIPE_INPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const MAX_RECIPE_ACTIONS: usize = 64;
const MAX_RECIPE_VALIDATIONS: usize = 64;
const MAX_RECIPE_SOURCE_EXPORTS: usize = 32;
const MAX_RECIPE_COMMAND_ARGUMENTS: usize = 1_024;
const MAX_RECIPE_ARGUMENT_BYTES: usize = 128 * 1024;
const MAX_RECIPE_NETWORK_AUTHORITIES: usize = 256;
const MAX_RECIPE_ENVIRONMENT_ENTRIES: usize = 256;
const MAX_RECIPE_VALIDATION_PARAMETERS: usize = 256;
const MAX_RECIPE_CHILD_PROCESSES: u32 = 256;

#[cfg(test)]
thread_local! {
    static COMMAND_RECIPE_LOAD_COUNT: Cell<u64> = const { Cell::new(0) };
}

pub(crate) static COMMAND_RECIPE_ADAPTER_METADATA: WorkspaceEnvironmentAdapterMetadata =
    WorkspaceEnvironmentAdapterMetadata {
        canonical_identity: RECIPE_ADAPTER_IDENTITY,
        namespace: "trail",
        name: "command",
        contract_major: 1,
        implementation_version: env!("CARGO_PKG_VERSION"),
        distribution_digest: "builtin:command-recipe-plan-v1",
        selectors: &[RECIPE_ADAPTER_IDENTITY, "command"],
        kind: "generated",
        layer_adapter_name: "command",
        discovery_markers: &RECIPE_SPEC_PATHS,
        supported_operating_systems: &["linux", "macos", "windows"],
        supported_architectures: &["aarch64", "x86_64"],
        stability: "experimental",
        description: "Repository-declared argv command with exact inputs, a contained generated output, denied network, and host sandbox enforcement",
    };

#[derive(Clone, Debug)]
struct CommandRecipe {
    schema: RecipeSchemaVersion,
    specification_digest: String,
    specification_sources: BTreeMap<String, String>,
    profile_versions: BTreeMap<String, String>,
    defaults: RecipeEnvironment,
    component: RecipeComponent,
}

#[derive(Clone, Debug)]
pub(crate) struct CompiledRepositoryArtifactPipelineV2 {
    pub(crate) proposal: EnvironmentDiscoveredComponentReport,
    pub(crate) resolution_plan: Option<ArtifactResolutionPlanV1>,
    pub(crate) graph_plan: WorkspaceEnvironmentPlan,
    pub(crate) desired_material: ArtifactDesiredKeyMaterialV2,
    pub(crate) desired_key: ArtifactDesiredKeyV2,
    pub(crate) outputs: Vec<ArtifactOutputContractV2>,
    pub(crate) validations: Vec<ArtifactValidationV1>,
    pub(crate) source_exports: Vec<ArtifactSourceExportContractV2>,
}

impl CompiledRepositoryArtifactPipelineV2 {
    fn into_graph_plan(self) -> Result<WorkspaceEnvironmentPlan> {
        if self.proposal.component_id != self.graph_plan.component_id
            || self.desired_material.component_id != self.graph_plan.component_id
            || self
                .resolution_plan
                .as_ref()
                .is_some_and(|plan| plan.component_id != self.graph_plan.component_id)
            || self.desired_material.outputs != self.outputs
            || self.desired_material.validations != self.validations
            || self.desired_material.source_exports != self.source_exports
            || super::workspace_artifact::artifact_desired_key_v2(self.desired_material)?
                != self.desired_key
        {
            return Err(Error::Corrupt(
                "compiled repository artifact pipeline models disagree".into(),
            ));
        }
        Ok(self.graph_plan)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RecipeSchemaVersion {
    V1,
    V2,
}

impl RecipeSchemaVersion {
    fn parse(value: &str, path: &str) -> Result<Self> {
        match value {
            RECIPE_SCHEMA_V1 => Ok(Self::V1),
            RECIPE_SCHEMA_V2 => Ok(Self::V2),
            other => Err(Error::InvalidInput(format!(
                "unsupported environment schema `{other}` in `{path}`; expected `{RECIPE_SCHEMA_V1}` or `{RECIPE_SCHEMA_V2}`"
            ))),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::V1 => RECIPE_SCHEMA_V1,
            Self::V2 => RECIPE_SCHEMA_V2,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeSpecification {
    schema: String,
    #[serde(default)]
    include: Vec<String>,
    #[serde(default)]
    environment: RecipeEnvironment,
    #[serde(default)]
    profile: BTreeMap<String, RecipeProfile>,
    #[serde(default, rename = "component")]
    components: Vec<RecipeComponentDefinition>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
struct RecipeEnvironment {
    name: Option<String>,
    default_network: String,
    default_scripts: String,
    #[serde(default)]
    missing_resolution: Option<String>,
}

impl Default for RecipeEnvironment {
    fn default() -> Self {
        Self {
            name: None,
            default_network: "deny".to_string(),
            default_scripts: "deny".to_string(),
            missing_resolution: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeComponentDefinition {
    id: String,
    #[serde(default)]
    root: String,
    #[serde(default)]
    extends: Vec<String>,
    #[serde(default)]
    adapter: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default, alias = "dependencies")]
    depends_on: Vec<String>,
    #[serde(default, rename = "edge")]
    edges: Vec<RecipeDependencyEdge>,
    #[serde(default, alias = "inputs", rename = "input")]
    inputs: Vec<RecipeInput>,
    #[serde(default, alias = "outputs", rename = "output")]
    outputs: Vec<RecipeOutput>,
    #[serde(default)]
    build: Option<RecipeBuild>,
    #[serde(default, rename = "resolve")]
    resolution: Option<RecipeResolution>,
    #[serde(default, rename = "action")]
    actions: Vec<RecipeAction>,
    #[serde(default, rename = "validation")]
    validations: Vec<RecipeValidation>,
    #[serde(default)]
    capabilities: Option<RecipeCapabilities>,
    #[serde(default, rename = "source_export")]
    source_exports: Vec<RecipeSourceExport>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeProfile {
    version: String,
    #[serde(default)]
    extends: Vec<String>,
    #[serde(default)]
    adapter: Option<String>,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default, alias = "dependencies")]
    depends_on: Vec<String>,
    #[serde(default, rename = "edge")]
    edges: Vec<RecipeDependencyEdge>,
    #[serde(default, alias = "inputs", rename = "input")]
    inputs: Vec<RecipeInput>,
    #[serde(default, alias = "outputs", rename = "output")]
    outputs: Vec<RecipeOutput>,
    #[serde(default)]
    build: Option<RecipeBuild>,
    #[serde(default, rename = "resolve")]
    resolution: Option<RecipeResolution>,
    #[serde(default, rename = "action")]
    actions: Vec<RecipeAction>,
    #[serde(default, rename = "validation")]
    validations: Vec<RecipeValidation>,
    #[serde(default)]
    capabilities: Option<RecipeCapabilities>,
    #[serde(default, rename = "source_export")]
    source_exports: Vec<RecipeSourceExport>,
}

#[derive(Clone, Debug, Default)]
struct RecipeFragment {
    adapter: Option<String>,
    kind: Option<String>,
    dependencies: Vec<String>,
    edges: Vec<RecipeDependencyEdge>,
    inputs: Vec<RecipeInput>,
    outputs: Vec<RecipeOutput>,
    build: Option<RecipeBuild>,
    resolution: Option<RecipeResolution>,
    actions: Vec<RecipeAction>,
    validations: Vec<RecipeValidation>,
    capabilities: Option<RecipeCapabilities>,
    source_exports: Vec<RecipeSourceExport>,
}

#[derive(Clone, Debug)]
struct ResolvedRecipeProfile {
    fragment: RecipeFragment,
    versions: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
struct RecipeDocuments {
    schema: Option<RecipeSchemaVersion>,
    defaults: RecipeEnvironment,
    profiles: BTreeMap<String, RecipeProfile>,
    components: Vec<RecipeComponentDefinition>,
    specification_sources: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Serialize)]
struct RecipeComponent {
    id: String,
    adapter: String,
    root: String,
    kind: String,
    dependencies: Vec<String>,
    edges: Vec<RecipeDependencyEdge>,
    inputs: Vec<RecipeInput>,
    outputs: Vec<RecipeOutput>,
    build: RecipeBuild,
    resolution: Option<RecipeResolution>,
    actions: Vec<RecipeAction>,
    validations: Vec<RecipeValidation>,
    capabilities: Option<RecipeCapabilities>,
    source_exports: Vec<RecipeSourceExport>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecipeInput {
    path: String,
    #[serde(default = "default_identity_role")]
    role: String,
    #[serde(default = "default_bytes_format")]
    format: String,
    #[serde(default)]
    optional: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecipeDependencyEdge {
    component: String,
    #[serde(rename = "type")]
    edge_type: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecipeOutput {
    #[serde(default)]
    name: Option<String>,
    source: String,
    target: String,
    #[serde(default)]
    policy: EnvironmentOutputPolicy,
    #[serde(default)]
    reuse: Option<EnvironmentReuseMode>,
    #[serde(default)]
    scope: Option<EnvironmentSharingScope>,
    #[serde(default)]
    publish: Option<EnvironmentPublicationTrigger>,
    #[serde(default)]
    gate: Option<String>,
    #[serde(default = "default_host_portability")]
    portability: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecipeBuild {
    command: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    network: Option<String>,
    #[serde(default)]
    scripts: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecipeResolution {
    command: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    network: Option<RecipeNetwork>,
    snapshot: String,
    format: String,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    capabilities: Option<RecipeCapabilities>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(untagged)]
enum RecipeNetwork {
    Policy(String),
    Authorities(RecipeNetworkAuthorities),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecipeNetworkAuthorities {
    authorities: Vec<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum RecipeActionPhase {
    Construct,
    Validate,
    MountedExecution,
    SourceExport,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecipeAction {
    #[serde(default)]
    name: Option<String>,
    phase: RecipeActionPhase,
    command: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    network: Option<RecipeNetwork>,
    #[serde(default)]
    scripts: Option<String>,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default)]
    capabilities: Option<RecipeCapabilities>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecipeValidation {
    #[serde(default)]
    name: Option<String>,
    kind: String,
    #[serde(default)]
    path: Option<String>,
    #[serde(default)]
    command: Vec<String>,
    #[serde(default = "default_true")]
    required: bool,
    #[serde(default)]
    parameters: BTreeMap<String, String>,
    #[serde(default)]
    gate: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecipeCapabilities {
    #[serde(default)]
    network: Option<String>,
    #[serde(default)]
    filesystem_read: Option<String>,
    #[serde(default)]
    filesystem_write: Option<String>,
    #[serde(default)]
    process: Option<String>,
    #[serde(default)]
    child_processes: Option<u32>,
    #[serde(default)]
    secrets: Option<String>,
    #[serde(default)]
    publication: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct RecipeSourceExport {
    #[serde(default)]
    name: Option<String>,
    from_output: String,
    source: String,
    target: String,
    mode: String,
    #[serde(default = "default_fail_collision")]
    collision: String,
    #[serde(default)]
    validation: Option<String>,
    #[serde(default)]
    gate: Option<String>,
}

fn default_recipe_kind() -> String {
    "generated".to_string()
}

fn default_identity_role() -> String {
    "identity".to_string()
}

fn default_bytes_format() -> String {
    "bytes".to_string()
}

fn default_host_portability() -> String {
    "host".to_string()
}

fn default_true() -> bool {
    true
}

fn default_fail_collision() -> String {
    "fail".to_string()
}

fn compile_recipe_validations(
    validations: &[RecipeValidation],
) -> Result<Vec<ArtifactValidationV1>> {
    let mut compiled = validations
        .iter()
        .enumerate()
        .map(|(index, validation)| {
            let kind = match validation.kind.as_str() {
                "structural" | "path_contract" => ArtifactValidationKindV1::Structural,
                "loadability" => ArtifactValidationKindV1::Loadability,
                "framework" => ArtifactValidationKindV1::Framework,
                "policy" => ArtifactValidationKindV1::Policy,
                "gate" => ArtifactValidationKindV1::Gate,
                "reproducibility" => ArtifactValidationKindV1::Reproducibility,
                other => {
                    return Err(Error::InvalidInput(format!(
                        "unsupported repository validation kind `{other}`"
                    )))
                }
            };
            let mut parameters = validation.parameters.clone();
            if let Some(path) = &validation.path {
                parameters.insert("path".into(), normalize_relative_path(path)?);
            }
            if !validation.command.is_empty() {
                parameters.insert(
                    "command".into(),
                    serde_json::to_string(&validation.command)?,
                );
            }
            if let Some(gate) = &validation.gate {
                parameters.insert("gate".into(), gate.clone());
            }
            Ok(ArtifactValidationV1 {
                name: validation
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("validation-{index}")),
                kind,
                required: validation.required,
                parameters,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    compiled.sort();
    if compiled.windows(2).any(|pair| pair[0].name == pair[1].name) {
        return Err(Error::InvalidInput(
            "repository component declares duplicate validation names".into(),
        ));
    }
    Ok(compiled)
}

fn compile_recipe_source_exports(
    component: &RecipeComponent,
) -> Vec<ArtifactSourceExportContractV2> {
    component
        .source_exports
        .iter()
        .map(|export| ArtifactSourceExportContractV2 {
            name: export
                .name
                .clone()
                .unwrap_or_else(|| export.from_output.clone()),
            output_name: export.from_output.clone(),
            artifact_subpath: export.source.clone(),
            destination: export.target.clone(),
            collision_policy: export.collision.clone(),
            required_validation: export.validation.clone().unwrap_or_else(|| {
                super::workspace_artifact::HOST_WORKSPACE_LAYER_STRUCTURAL_SEAL.into()
            }),
            required_gate: export.gate.clone(),
            authorization_mode: export.mode.clone(),
        })
        .collect()
}

fn recipe_network_authorities(network: Option<&RecipeNetwork>) -> Result<Vec<String>> {
    let mut authorities = match network {
        None => Vec::new(),
        Some(RecipeNetwork::Policy(policy)) if policy == "deny" => Vec::new(),
        Some(RecipeNetwork::Policy(policy)) => Err(Error::InvalidInput(format!(
            "repository resolver network policy `{policy}` must be `deny` or an exact authority list"
        )))?,
        Some(RecipeNetwork::Authorities(authorities)) => authorities.authorities.clone(),
    };
    if authorities.len() > MAX_RECIPE_NETWORK_AUTHORITIES {
        return Err(Error::InvalidInput(format!(
            "repository resolver declares more than {MAX_RECIPE_NETWORK_AUTHORITIES} network authorities"
        )));
    }
    for authority in &authorities {
        validate_recipe_network_authority(authority)?;
    }
    authorities.sort();
    authorities.dedup();
    Ok(authorities)
}

fn recipe_network_policy_identity(network: Option<&RecipeNetwork>) -> Result<String> {
    let authorities = recipe_network_authorities(network)?;
    if authorities.is_empty() {
        Ok("deny".into())
    } else {
        Ok(format!("exact:{}", authorities.join(",")))
    }
}

fn merge_recipe_identity_environment(
    target: &mut BTreeMap<String, String>,
    source: &BTreeMap<String, String>,
    component_id: &str,
) -> Result<()> {
    for (name, value) in source {
        validate_recipe_environment(name, value, component_id)?;
        if let Some(previous) = target.insert(name.clone(), value.clone())
            && previous != *value
        {
            return Err(Error::InvalidInput(format!(
                "repository component `{component_id}` declares conflicting identity environment values for `{name}`"
            )));
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum RecipeCapabilityPhase {
    Resolve,
    Construct,
    Validate,
}

impl RecipeCapabilityPhase {
    fn name(self) -> &'static str {
        match self {
            Self::Resolve => "resolve",
            Self::Construct => "construct",
            Self::Validate => "validate",
        }
    }
}

fn validate_recipe_v2_component(
    component: &RecipeComponent,
    defaults: &RecipeEnvironment,
) -> Result<()> {
    if component.inputs.len() > MAX_RECIPE_INPUT_DECLARATIONS {
        return Err(Error::InvalidInput(format!(
            "repository component `{}` declares more than {MAX_RECIPE_INPUT_DECLARATIONS} inputs",
            component.id
        )));
    }
    if component.actions.len() > MAX_RECIPE_ACTIONS {
        return Err(Error::InvalidInput(format!(
            "repository component `{}` declares more than {MAX_RECIPE_ACTIONS} actions",
            component.id
        )));
    }
    if component.validations.len() > MAX_RECIPE_VALIDATIONS {
        return Err(Error::InvalidInput(format!(
            "repository component `{}` declares more than {MAX_RECIPE_VALIDATIONS} validations",
            component.id
        )));
    }
    if component.source_exports.len() > MAX_RECIPE_SOURCE_EXPORTS {
        return Err(Error::InvalidInput(format!(
            "repository component `{}` declares more than {MAX_RECIPE_SOURCE_EXPORTS} source exports",
            component.id
        )));
    }

    validate_recipe_fixed_argv(
        &component.build.command,
        &component.id,
        "build.command",
        false,
    )?;
    validate_recipe_phase_cwd(
        component.build.cwd.as_deref().unwrap_or(&component.root),
        &component.root,
        &component.id,
        "build.cwd",
    )?;
    validate_recipe_environment_map(&component.build.environment, &component.id, "build")?;
    if component
        .build
        .network
        .as_deref()
        .unwrap_or(&defaults.default_network)
        != "deny"
        || component
            .build
            .scripts
            .as_deref()
            .unwrap_or(&defaults.default_scripts)
            != "deny"
    {
        return Err(Error::InvalidInput(format!(
            "repository component `{}` build requires network = \"deny\" and scripts = \"deny\"",
            component.id
        )));
    }
    if let Some(capabilities) = &component.capabilities {
        validate_recipe_capabilities(
            capabilities,
            RecipeCapabilityPhase::Construct,
            &component.id,
        )?;
    }

    if let Some(resolution) = &component.resolution {
        validate_recipe_fixed_argv(&resolution.command, &component.id, "resolve.command", false)?;
        validate_recipe_phase_cwd(
            resolution.cwd.as_deref().unwrap_or(&component.root),
            &component.root,
            &component.id,
            "resolve.cwd",
        )?;
        normalize_relative_path(&resolution.snapshot)?;
        if resolution.format.is_empty()
            || resolution.format.len() > 512
            || resolution.format.contains(char::is_control)
            || contains_sensitive_text(&resolution.format)
        {
            return Err(Error::InvalidInput(format!(
                "repository component `{}` has an invalid resolution snapshot format",
                component.id
            )));
        }
        recipe_network_authorities(resolution.network.as_ref())?;
        validate_recipe_environment_map(&resolution.environment, &component.id, "resolver")?;
        if let Some(capabilities) = &resolution.capabilities {
            validate_recipe_capabilities(
                capabilities,
                RecipeCapabilityPhase::Resolve,
                &component.id,
            )?;
        }
    }

    let mut action_names = BTreeSet::new();
    for (index, action) in component.actions.iter().enumerate() {
        if matches!(
            action.phase,
            RecipeActionPhase::MountedExecution | RecipeActionPhase::SourceExport
        ) {
            return Err(Error::InvalidInput(format!(
                "repository component `{}` action {} requests forbidden phase `{:?}`; mounted execution and source export cannot execute repository commands",
                component.id, index, action.phase
            )));
        }
        let action_name = action
            .name
            .clone()
            .unwrap_or_else(|| format!("action-{index}"));
        validate_recipe_output_name(&action_name, &component.id)?;
        if !action_names.insert(action_name.clone()) {
            return Err(Error::InvalidInput(format!(
                "repository component `{}` declares action name `{action_name}` more than once",
                component.id
            )));
        }
        validate_recipe_fixed_argv(
            &action.command,
            &component.id,
            &format!("action `{action_name}` command"),
            false,
        )?;
        validate_recipe_phase_cwd(
            action.cwd.as_deref().unwrap_or(&component.root),
            &component.root,
            &component.id,
            &format!("action `{action_name}` cwd"),
        )?;
        if !recipe_network_authorities(action.network.as_ref())?.is_empty() {
            return Err(Error::InvalidInput(format!(
                "repository component `{}` action `{action_name}` must be offline",
                component.id
            )));
        }
        if action
            .scripts
            .as_deref()
            .is_some_and(|policy| policy != "deny")
        {
            return Err(Error::InvalidInput(format!(
                "repository component `{}` action `{action_name}` must deny scripts",
                component.id
            )));
        }
        validate_recipe_environment_map(
            &action.environment,
            &component.id,
            &format!("action `{action_name}`"),
        )?;
        if let Some(capabilities) = &action.capabilities {
            let phase = if action.phase == RecipeActionPhase::Validate {
                RecipeCapabilityPhase::Validate
            } else {
                RecipeCapabilityPhase::Construct
            };
            validate_recipe_capabilities(capabilities, phase, &component.id)?;
        }
    }

    for (index, validation) in component.validations.iter().enumerate() {
        if let Some(name) = &validation.name {
            validate_recipe_output_name(name, &component.id)?;
        }
        if let Some(path) = &validation.path {
            normalize_relative_path(path)?;
        }
        validate_recipe_fixed_argv(
            &validation.command,
            &component.id,
            &format!("validation {index} command"),
            true,
        )?;
        if validation.parameters.len() > MAX_RECIPE_VALIDATION_PARAMETERS {
            return Err(Error::InvalidInput(format!(
                "repository component `{}` validation {index} declares too many parameters",
                component.id
            )));
        }
        for (name, value) in &validation.parameters {
            if name.is_empty()
                || name.len() > 128
                || value.len() > MAX_RECIPE_ARGUMENT_BYTES
                || name.contains(char::is_control)
                || value.contains(char::is_control)
                || contains_sensitive_text(name)
                || contains_sensitive_text(value)
                || contains_provider_socket_reference(value)
            {
                return Err(Error::InvalidInput(format!(
                    "repository component `{}` validation {index} has an unsafe parameter `{name}`",
                    component.id
                )));
            }
        }
    }

    for output in &component.outputs {
        if output.reuse == Some(EnvironmentReuseMode::Compatible)
            || output.scope == Some(EnvironmentSharingScope::Host)
        {
            return Err(Error::InvalidInput(format!(
                "repository component `{}` cannot request compatible or host-wide artifact reuse",
                component.id
            )));
        }
    }
    let output_names = component
        .outputs
        .iter()
        .enumerate()
        .map(|(index, output)| {
            output
                .name
                .clone()
                .unwrap_or_else(|| format!("output-{index}"))
        })
        .collect::<BTreeSet<_>>();
    for export in &component.source_exports {
        let export_name = export.name.as_deref().unwrap_or(&export.from_output);
        validate_recipe_output_name(export_name, &component.id)?;
        validate_recipe_output_name(&export.from_output, &component.id)?;
        if !output_names.contains(&export.from_output) {
            return Err(Error::InvalidInput(format!(
                "repository component `{}` source export references unknown output `{}`",
                component.id, export.from_output
            )));
        }
        normalize_relative_path(&export.source)?;
        normalize_relative_path(&export.target)?;
        if export.mode != "explicit" {
            return Err(Error::InvalidInput(format!(
                "repository component `{}` source export mode must be `explicit`",
                component.id
            )));
        }
        if !matches!(export.collision.as_str(), "fail" | "replace") {
            return Err(Error::InvalidInput(format!(
                "repository component `{}` source export `{export_name}` collision mode must be `fail` or `replace`",
                component.id
            )));
        }
        if let Some(validation) = &export.validation {
            validate_recipe_output_name(validation, &component.id)?;
        }
        if let Some(gate) = &export.gate {
            validate_recipe_output_name(gate, &component.id)?;
        }
    }
    let mut export_names = component
        .source_exports
        .iter()
        .map(|export| export.name.as_deref().unwrap_or(&export.from_output))
        .collect::<Vec<_>>();
    export_names.sort_unstable();
    if export_names.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(Error::InvalidInput(format!(
            "repository component `{}` declares duplicate source export names",
            component.id
        )));
    }
    Ok(())
}

fn validate_recipe_fixed_argv(
    command: &[String],
    component_id: &str,
    field: &str,
    allow_empty: bool,
) -> Result<()> {
    if command.is_empty() {
        if allow_empty {
            return Ok(());
        }
        return Err(Error::InvalidInput(format!(
            "repository component `{component_id}` has an empty {field}"
        )));
    }
    if command.len() > MAX_RECIPE_COMMAND_ARGUMENTS {
        return Err(Error::InvalidInput(format!(
            "repository component `{component_id}` {field} exceeds {MAX_RECIPE_COMMAND_ARGUMENTS} argv entries"
        )));
    }
    let program = &command[0];
    if program.contains('/')
        || program.contains('\\')
        || is_shell_program(program)
        || is_indirect_process_launcher(program)
    {
        return Err(Error::InvalidInput(format!(
            "repository component `{component_id}` {field} must name one non-shell, non-launcher executable from PATH, not `{program}`"
        )));
    }
    for argument in command {
        if argument.is_empty()
            || argument.len() > MAX_RECIPE_ARGUMENT_BYTES
            || argument.contains('\0')
            || argument.contains('\n')
            || argument.contains('\r')
            || contains_sensitive_text(argument)
            || contains_shell_interpolation(argument)
            || contains_provider_socket_reference(argument)
            || is_absolute_host_path(argument)
        {
            return Err(Error::InvalidInput(format!(
                "repository component `{component_id}` {field} contains an unsafe or excessive argv entry"
            )));
        }
    }
    Ok(())
}

fn validate_recipe_phase_cwd(
    cwd: &str,
    component_root: &str,
    component_id: &str,
    field: &str,
) -> Result<()> {
    let cwd = normalize_recipe_path_allow_root(cwd)?;
    if !component_root.is_empty()
        && cwd != component_root
        && !cwd.starts_with(&format!("{component_root}/"))
    {
        return Err(Error::InvalidInput(format!(
            "repository component `{component_id}` {field} `{cwd}` escapes component root `{component_root}`"
        )));
    }
    Ok(())
}

fn validate_recipe_environment_map(
    environment: &BTreeMap<String, String>,
    component_id: &str,
    phase: &str,
) -> Result<()> {
    if environment.len() > MAX_RECIPE_ENVIRONMENT_ENTRIES {
        return Err(Error::InvalidInput(format!(
            "repository component `{component_id}` {phase} environment exceeds {MAX_RECIPE_ENVIRONMENT_ENTRIES} entries"
        )));
    }
    for (name, value) in environment {
        validate_recipe_environment(name, value, component_id)?;
        if contains_provider_socket_reference(name) || contains_provider_socket_reference(value) {
            return Err(Error::InvalidInput(format!(
                "repository component `{component_id}` {phase} environment entry `{name}` requests a provider socket"
            )));
        }
    }
    Ok(())
}

fn validate_recipe_capabilities(
    capabilities: &RecipeCapabilities,
    phase: RecipeCapabilityPhase,
    component_id: &str,
) -> Result<()> {
    let allowed_network = match phase {
        RecipeCapabilityPhase::Resolve => &["deny", "exact_authorities"][..],
        RecipeCapabilityPhase::Construct | RecipeCapabilityPhase::Validate => &["deny"][..],
    };
    let allowed_read = match phase {
        RecipeCapabilityPhase::Resolve | RecipeCapabilityPhase::Construct => {
            &["declared_inputs"][..]
        }
        RecipeCapabilityPhase::Validate => &["artifact_candidate"][..],
    };
    let allowed_write = match phase {
        RecipeCapabilityPhase::Resolve | RecipeCapabilityPhase::Construct => {
            &["isolated_candidate"][..]
        }
        RecipeCapabilityPhase::Validate => &["validation_receipt"][..],
    };
    validate_recipe_capability_value(
        capabilities.network.as_deref(),
        allowed_network,
        component_id,
        phase,
        "network",
    )?;
    validate_recipe_capability_value(
        capabilities.filesystem_read.as_deref(),
        allowed_read,
        component_id,
        phase,
        "filesystem_read",
    )?;
    validate_recipe_capability_value(
        capabilities.filesystem_write.as_deref(),
        allowed_write,
        component_id,
        phase,
        "filesystem_write",
    )?;
    validate_recipe_capability_value(
        capabilities.process.as_deref(),
        &["declared_executable"],
        component_id,
        phase,
        "process",
    )?;
    let allowed_secrets = if matches!(phase, RecipeCapabilityPhase::Resolve) {
        &["deny", "opaque_handles"][..]
    } else {
        &["deny"][..]
    };
    validate_recipe_capability_value(
        capabilities.secrets.as_deref(),
        allowed_secrets,
        component_id,
        phase,
        "secrets",
    )?;
    validate_recipe_capability_value(
        capabilities.publication.as_deref(),
        &["deny"],
        component_id,
        phase,
        "publication",
    )?;
    if capabilities
        .child_processes
        .is_some_and(|limit| limit == 0 || limit > MAX_RECIPE_CHILD_PROCESSES)
    {
        return Err(Error::InvalidInput(format!(
            "repository component `{component_id}` {} child-process limit must be between 1 and {MAX_RECIPE_CHILD_PROCESSES}",
            phase.name()
        )));
    }
    Ok(())
}

fn validate_recipe_capability_value(
    value: Option<&str>,
    allowed: &[&str],
    component_id: &str,
    phase: RecipeCapabilityPhase,
    field: &str,
) -> Result<()> {
    if let Some(value) = value
        && !allowed.contains(&value)
    {
        return Err(Error::InvalidInput(format!(
            "repository component `{component_id}` {} capability `{field} = {value}` exceeds the repository-declaration ceiling",
            phase.name()
        )));
    }
    Ok(())
}

fn validate_recipe_network_authority(authority: &str) -> Result<()> {
    if authority.is_empty()
        || authority.len() > 512
        || authority.contains(char::is_whitespace)
        || authority.contains(char::is_control)
        || authority.contains('/')
        || authority.contains('\\')
        || authority.contains('@')
        || authority.contains("//")
        || contains_sensitive_text(authority)
        || contains_shell_interpolation(authority)
        || contains_provider_socket_reference(authority)
    {
        return Err(Error::InvalidInput(format!(
            "repository resolver authority `{authority}` is not an exact non-secret network authority"
        )));
    }
    Ok(())
}

fn contains_shell_interpolation(argument: &str) -> bool {
    argument.contains("$(")
        || argument.contains("${")
        || argument.contains('`')
        || matches!(
            argument,
            "&&" | "||" | ";" | "|" | "&" | ">" | ">>" | "<" | "<<"
        )
}

fn is_indirect_process_launcher(program: &str) -> bool {
    matches!(
        program.to_ascii_lowercase().as_str(),
        "env"
            | "xargs"
            | "parallel"
            | "nohup"
            | "nice"
            | "setsid"
            | "sudo"
            | "su"
            | "doas"
            | "command"
            | "exec"
    )
}

fn is_absolute_host_path(argument: &str) -> bool {
    argument.starts_with('/')
        || argument.starts_with("\\\\")
        || argument.starts_with("file://")
        || argument.as_bytes().get(1) == Some(&b':')
            && argument
                .as_bytes()
                .first()
                .is_some_and(u8::is_ascii_alphabetic)
            && argument
                .as_bytes()
                .get(2)
                .is_some_and(|byte| matches!(byte, b'/' | b'\\'))
}

fn contains_provider_socket_reference(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    lower.contains("unix://")
        || lower.contains("npipe://")
        || lower.contains("/var/run/docker.sock")
        || lower.contains("/run/docker.sock")
        || lower.contains("/run/containerd/")
        || lower.contains("ssh_auth_sock")
        || lower.contains("docker_host")
        || lower.contains("container_host")
        || lower.contains("buildkit_host")
}

impl Trail {
    pub(crate) fn command_recipe_discovery(
        &self,
        source_root: &ObjectId,
        component_root: Option<&str>,
    ) -> Result<Vec<EnvironmentDiscoveredComponentReport>> {
        let requested_root = component_root
            .map(normalize_recipe_path_allow_root)
            .transpose()?;
        let recipes = self.load_command_recipes(source_root)?;
        Ok(recipes
            .into_iter()
            .filter(|recipe| {
                requested_root
                    .as_ref()
                    .is_none_or(|root| root == &recipe.component.root)
            })
            .map(|recipe| {
                let resolvable = recipe.schema == RecipeSchemaVersion::V2
                    && recipe.component.resolution.is_some();
                EnvironmentDiscoveredComponentReport {
                    component_id: recipe.component.id.clone(),
                    component_root: recipe.component.root,
                    kind: recipe.component.kind,
                    adapter_identity: RECIPE_ADAPTER_IDENTITY.to_string(),
                    status: if resolvable {
                        EnvironmentComponentProposalStatus::Resolvable
                    } else {
                        EnvironmentComponentProposalStatus::Ready
                    },
                    reasons: if resolvable {
                        vec![EnvironmentProposalReasonReport {
                            code: "resolution_snapshot_required".into(),
                            message:
                                "repository component declares an explicit resolution snapshot"
                                    .into(),
                        }]
                    } else {
                        Vec::new()
                    },
                    recovery_actions: if resolvable {
                        vec![EnvironmentRecoveryActionReport {
                            code: "resolve_component".into(),
                            description: "resolve and pin the declared component snapshot".into(),
                            command: Some(vec![
                                "trail".into(),
                                "env".into(),
                                "resolve".into(),
                                "--component".into(),
                                recipe.component.id,
                            ]),
                        }]
                    } else {
                        Vec::new()
                    },
                }
            })
            .collect())
    }

    pub(crate) fn command_recipe_plan(
        &self,
        source_root: &ObjectId,
        component_id: &str,
    ) -> Result<WorkspaceEnvironmentPlan> {
        let recipes = self.load_command_recipes(source_root)?;
        let recipe = recipes
            .into_iter()
            .find(|recipe| recipe.component.id == component_id)
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "no `{RECIPE_ADAPTER_IDENTITY}` component named `{component_id}` exists in the pinned environment specification"
                ))
            })?;
        if recipe.schema == RecipeSchemaVersion::V2 {
            return self
                .compile_repository_artifact_pipeline_v2_recipe(source_root, recipe)?
                .into_graph_plan();
        }
        self.plan_command_recipe(source_root, recipe)
    }

    pub(crate) fn command_recipe_resolution_plan(
        &self,
        source_root: &ObjectId,
        component_id: &str,
    ) -> Result<Option<ArtifactResolutionPlanV1>> {
        let recipe = self
            .load_command_recipes(source_root)?
            .into_iter()
            .find(|recipe| recipe.component.id == component_id)
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "no repository environment component named `{component_id}` exists"
                ))
            })?;
        if recipe.schema != RecipeSchemaVersion::V2 {
            return Ok(None);
        }
        Ok(self
            .compile_repository_artifact_pipeline_v2_recipe(source_root, recipe)?
            .resolution_plan)
    }

    #[cfg(test)]
    fn compile_repository_artifact_pipeline_v2(
        &self,
        source_root: &ObjectId,
        component_id: &str,
    ) -> Result<CompiledRepositoryArtifactPipelineV2> {
        let recipe = self
            .load_command_recipes(source_root)?
            .into_iter()
            .find(|recipe| recipe.component.id == component_id)
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "no repository environment component named `{component_id}` exists"
                ))
            })?;
        self.compile_repository_artifact_pipeline_v2_recipe(source_root, recipe)
    }

    fn compile_repository_artifact_pipeline_v2_recipe(
        &self,
        source_root: &ObjectId,
        recipe: CommandRecipe,
    ) -> Result<CompiledRepositoryArtifactPipelineV2> {
        if recipe.schema != RecipeSchemaVersion::V2 {
            return Err(Error::InvalidInput(format!(
                "component `{}` requires `{RECIPE_SCHEMA_V2}` for artifact-pipeline compilation",
                recipe.component.id
            )));
        }
        let graph_plan = self.plan_command_recipe(source_root, recipe.clone())?;
        let component = &recipe.component;
        let validations = compile_recipe_validations(&component.validations)?;
        let resolution_plan = component
            .resolution
            .as_ref()
            .map(|resolution| {
                self.compile_recipe_resolution_plan(
                    source_root,
                    &recipe,
                    &graph_plan,
                    resolution,
                    &validations,
                )
            })
            .transpose()?;
        let outputs = graph_plan
            .outputs
            .iter()
            .map(|output| ArtifactOutputContractV2 {
                name: output.name.clone(),
                output_path: output.output_path.clone(),
                mount_path: output.mount_path.clone(),
                policy: output.policy,
                reuse: output.reuse,
                scope: output.scope,
                publish: output.publish,
                gate: output.gate.clone(),
            })
            .collect::<Vec<_>>();
        let source_exports = compile_recipe_source_exports(component);
        let mut actions = Vec::new();
        actions.push(self.compile_recipe_action_identity(
            "build",
            ArtifactActionPhaseV2::Construct,
            &component.build.command,
            component.build.cwd.as_deref().unwrap_or("."),
            &component.build.environment,
        )?);
        if let Some(resolution) = &component.resolution {
            actions.push(self.compile_recipe_action_identity(
                "resolve",
                ArtifactActionPhaseV2::Resolve,
                &resolution.command,
                resolution.cwd.as_deref().unwrap_or("."),
                &resolution.environment,
            )?);
        }
        for (index, action) in component.actions.iter().enumerate() {
            let phase = match action.phase {
                RecipeActionPhase::Construct => ArtifactActionPhaseV2::Construct,
                RecipeActionPhase::Validate => ArtifactActionPhaseV2::Validate,
                RecipeActionPhase::MountedExecution | RecipeActionPhase::SourceExport => {
                    ArtifactActionPhaseV2::Finalize
                }
            };
            let action_name = action
                .name
                .clone()
                .unwrap_or_else(|| format!("action-{index}"));
            actions.push(self.compile_recipe_action_identity(
                &action_name,
                phase,
                &action.command,
                action.cwd.as_deref().unwrap_or(&component.root),
                &action.environment,
            )?);
        }
        actions.sort();

        let mut build_environment = component.build.environment.clone();
        if let Some(resolution) = &component.resolution {
            merge_recipe_identity_environment(
                &mut build_environment,
                &resolution.environment,
                &component.id,
            )?;
        }
        for action in &component.actions {
            merge_recipe_identity_environment(
                &mut build_environment,
                &action.environment,
                &component.id,
            )?;
        }
        let declared_inputs = graph_plan
            .inputs
            .iter()
            .map(|input| ArtifactResolutionInputV1 {
                source_path: input.source_path.clone(),
                content_hash: input.entry.content_hash.clone(),
                size_bytes: input.entry.size_bytes,
            })
            .collect::<Vec<_>>();
        let desired_material = ArtifactDesiredKeyMaterialV2 {
            version: 2,
            component_id: component.id.clone(),
            adapter_identity: component.adapter.clone(),
            adapter_implementation_version: env!("CARGO_PKG_VERSION").into(),
            adapter_distribution_digest: "builtin:repository-environment-v2".into(),
            adapter_protocol: RECIPE_SCHEMA_V2.into(),
            resolution_snapshot_id: None,
            source_closure: ArtifactSourceClosureV2 {
                normalizer_version: "repository-inputs/v1".into(),
                certified_complete: false,
                complete_source_root: Some(source_root.clone()),
                declared_inputs,
            },
            upstream_identities: BTreeMap::new(),
            actions,
            outputs: outputs.clone(),
            validations: validations.clone(),
            source_exports: source_exports.clone(),
            build_environment,
            target: "repository-declared".into(),
            platform: std::env::consts::OS.into(),
            architecture: std::env::consts::ARCH.into(),
            abi: "host-default".into(),
            // The host has normalized every output through
            // `validate_environment_output_contract`; repository text alone
            // never sets this bit and compatible reuse remains unavailable.
            portability_certified: true,
            portability_scope: "workspace".into(),
            trust_scope: "repository".into(),
            network_policy: recipe_network_policy_identity(
                component
                    .resolution
                    .as_ref()
                    .and_then(|resolution| resolution.network.as_ref()),
            )?,
            script_policy: ArtifactScriptPolicyV1::Deny,
            sandbox_policy: "restricted-repository-pipeline-v2".into(),
        };
        let desired_key =
            super::workspace_artifact::artifact_desired_key_v2(desired_material.clone())?;
        let proposal = EnvironmentDiscoveredComponentReport {
            component_id: component.id.clone(),
            component_root: component.root.clone(),
            kind: component.kind.clone(),
            adapter_identity: component.adapter.clone(),
            status: if resolution_plan.is_some() {
                EnvironmentComponentProposalStatus::Resolvable
            } else {
                EnvironmentComponentProposalStatus::Ready
            },
            reasons: resolution_plan.as_ref().map_or_else(Vec::new, |_| {
                vec![EnvironmentProposalReasonReport {
                    code: "resolution_snapshot_required".into(),
                    message: "repository component declares an explicit resolution snapshot".into(),
                }]
            }),
            recovery_actions: resolution_plan.as_ref().map_or_else(Vec::new, |_| {
                vec![EnvironmentRecoveryActionReport {
                    code: "resolve_component".into(),
                    description: "resolve and pin the declared component snapshot".into(),
                    command: Some(vec![
                        "trail".into(),
                        "env".into(),
                        "resolve".into(),
                        "--component".into(),
                        component.id.clone(),
                    ]),
                }]
            }),
        };
        Ok(CompiledRepositoryArtifactPipelineV2 {
            proposal,
            resolution_plan,
            graph_plan,
            desired_material,
            desired_key,
            outputs,
            validations,
            source_exports,
        })
    }

    fn compile_recipe_resolution_plan(
        &self,
        source_root: &ObjectId,
        recipe: &CommandRecipe,
        graph_plan: &WorkspaceEnvironmentPlan,
        resolution: &RecipeResolution,
        validations: &[ArtifactValidationV1],
    ) -> Result<ArtifactResolutionPlanV1> {
        let program = resolution.command.first().ok_or_else(|| {
            Error::InvalidInput(format!(
                "repository component `{}` resolver command is empty",
                recipe.component.id
            ))
        })?;
        let tool = resolve_workspace_tool_executable(program)?;
        let working_directory = resolution
            .cwd
            .clone()
            .unwrap_or_else(|| recipe.component.root.clone());
        let mut plan = ArtifactResolutionPlanV1 {
            version: ARTIFACT_RESOLUTION_PLAN_VERSION,
            proposal_key: format!("repository_v2_{}", recipe.specification_digest),
            source_root: source_root.clone(),
            component_id: recipe.component.id.clone(),
            adapter_identity: recipe.component.adapter.clone(),
            policy_identity: sha256_hex(&serde_json::to_vec(&(
                &resolution.capabilities,
                &recipe.component.capabilities,
            ))?),
            program: program.clone(),
            resolved_program: tool.path.to_string_lossy().into_owned(),
            executable_identity: tool.identity,
            argv: resolution.command.clone(),
            working_directory: working_directory.clone(),
            readable_inputs: graph_plan
                .inputs
                .iter()
                .map(|input| ArtifactResolutionInputV1 {
                    source_path: input.source_path.clone(),
                    content_hash: input.entry.content_hash.clone(),
                    size_bytes: input.entry.size_bytes,
                })
                .collect(),
            candidate_output: normalize_relative_path(&join_recipe_path(
                &working_directory,
                &resolution.snapshot,
            ))?,
            allowed_authorities: recipe_network_authorities(resolution.network.as_ref())?,
            credential_handles: Vec::new(),
            script_policy: ArtifactScriptPolicyV1::Deny,
            environment_roles: resolution
                .environment
                .keys()
                .map(|name| (name.clone(), ArtifactEnvironmentRoleV1::Identity))
                .collect(),
            limits: ArtifactActionLimitsV1 {
                timeout_ms: 5 * 60 * 1_000,
                stdout_bytes: 1024 * 1024,
                stderr_bytes: 1024 * 1024,
                candidate_bytes: 256 * 1024 * 1024,
                candidate_entries: 100_000,
                child_processes: resolution
                    .capabilities
                    .as_ref()
                    .and_then(|capabilities| capabilities.child_processes)
                    .unwrap_or(1)
                    .max(1),
            },
            snapshot_format: resolution.format.clone(),
            validations: if validations.is_empty() {
                vec![ArtifactValidationV1 {
                    name: "snapshot-structure".into(),
                    kind: ArtifactValidationKindV1::Structural,
                    required: true,
                    parameters: BTreeMap::new(),
                }]
            } else {
                validations.to_vec()
            },
        };
        super::workspace_artifact::normalize_artifact_resolution_plan(&mut plan)?;
        Ok(plan)
    }

    fn compile_recipe_action_identity(
        &self,
        name: &str,
        phase: ArtifactActionPhaseV2,
        command: &[String],
        working_directory: &str,
        environment: &BTreeMap<String, String>,
    ) -> Result<ArtifactActionIdentityV2> {
        let program = command.first().ok_or_else(|| {
            Error::InvalidInput(format!("repository action `{name}` command is empty"))
        })?;
        let tool = resolve_workspace_tool_executable(program)?;
        let normalized_working_directory = normalize_recipe_path_allow_root(working_directory)?;
        Ok(ArtifactActionIdentityV2 {
            name: name.into(),
            phase,
            executable_identity: tool.identity,
            argv: command.to_vec(),
            working_directory: if normalized_working_directory.is_empty() {
                ".".into()
            } else {
                normalized_working_directory
            },
            environment_names: environment.keys().cloned().collect(),
        })
    }

    pub(crate) fn command_recipe_plans(
        &self,
        source_root: &ObjectId,
        component_ids: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, WorkspaceEnvironmentPlan>> {
        let recipes = self.load_command_recipes(source_root)?;
        let mut plans = BTreeMap::new();
        let mut tools = BTreeMap::<String, ResolvedWorkspaceTool>::new();
        for recipe in recipes {
            if component_ids.contains(&recipe.component.id) {
                let component_id = recipe.component.id.clone();
                if recipe.schema == RecipeSchemaVersion::V2 {
                    plans.insert(
                        component_id,
                        self.compile_repository_artifact_pipeline_v2_recipe(source_root, recipe)?
                            .into_graph_plan()?,
                    );
                    continue;
                }
                let program = recipe
                    .component
                    .build
                    .command
                    .first()
                    .cloned()
                    .ok_or_else(|| {
                        Error::InvalidInput(format!(
                            "command component `{component_id}` has an empty build.command"
                        ))
                    })?;
                let tool = if let Some(tool) = tools.get(&program) {
                    tool.clone()
                } else {
                    let tool = resolve_workspace_tool_executable(&program)?;
                    tools.insert(program, tool.clone());
                    tool
                };
                plans.insert(
                    component_id,
                    self.plan_command_recipe_with_tool(source_root, recipe, Some(tool))?,
                );
            }
        }
        if plans.len() != component_ids.len() {
            let missing = component_ids
                .iter()
                .filter(|component_id| !plans.contains_key(*component_id))
                .cloned()
                .collect::<Vec<_>>();
            return Err(Error::InvalidInput(format!(
                "pinned environment specification is missing command component(s): {}",
                missing.join(", ")
            )));
        }
        Ok(plans)
    }

    pub(crate) fn command_recipe_source_exports(
        &self,
        source_root: &ObjectId,
        component_id: &str,
    ) -> Result<Vec<ArtifactSourceExportContractV2>> {
        let recipe = self
            .load_command_recipes(source_root)?
            .into_iter()
            .find(|recipe| recipe.component.id == component_id)
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "no repository environment component named `{component_id}` exists"
                ))
            })?;
        if recipe.schema != RecipeSchemaVersion::V2 {
            return Err(Error::InvalidInput(format!(
                "component `{component_id}` requires `{RECIPE_SCHEMA_V2}` for source export"
            )));
        }
        Ok(compile_recipe_source_exports(&recipe.component))
    }

    pub(crate) fn command_recipe_plan_for_root(
        &self,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<WorkspaceEnvironmentPlan> {
        let component_root = normalize_recipe_path_allow_root(component_root)?;
        let mut matching = self
            .load_command_recipes(source_root)?
            .into_iter()
            .filter(|recipe| recipe.component.root == component_root)
            .collect::<Vec<_>>();
        match matching.len() {
            1 => {
                let recipe = matching.remove(0);
                if recipe.schema == RecipeSchemaVersion::V2 {
                    self.compile_repository_artifact_pipeline_v2_recipe(source_root, recipe)?
                        .into_graph_plan()
                } else {
                    self.plan_command_recipe(source_root, recipe)
                }
            }
            0 => Err(Error::InvalidInput(format!(
                "no `{RECIPE_ADAPTER_IDENTITY}` component is declared at `{}`",
                display_recipe_root(&component_root)
            ))),
            count => Err(Error::InvalidInput(format!(
                "{count} `{RECIPE_ADAPTER_IDENTITY}` components are declared at `{}`; synchronize all components or give each recipe a distinct root",
                display_recipe_root(&component_root)
            ))),
        }
    }

    fn load_command_recipes(&self, source_root: &ObjectId) -> Result<Vec<CommandRecipe>> {
        #[cfg(test)]
        COMMAND_RECIPE_LOAD_COUNT.with(|count| count.set(count.get() + 1));
        let mut found = Vec::new();
        for path in RECIPE_SPEC_PATHS {
            if self.root_file_entry(source_root, path)?.is_some() {
                found.push(path.to_string());
            }
        }
        if found.len() > 1 {
            return Err(Error::InvalidInput(format!(
                "environment specification is ambiguous; keep only one of {}",
                RECIPE_SPEC_PATHS.join(", ")
            )));
        }
        let Some(spec_path) = found.pop() else {
            return Ok(Vec::new());
        };

        let mut documents = RecipeDocuments::default();
        let mut visited = BTreeSet::new();
        let mut stack = Vec::new();
        let mut total_bytes = 0u64;
        self.collect_recipe_document(
            source_root,
            &spec_path,
            0,
            true,
            &mut documents,
            &mut visited,
            &mut stack,
            &mut total_bytes,
        )?;

        let mut ids = BTreeSet::new();
        let mut targets = BTreeMap::<String, String>::new();
        let mut profile_cache = BTreeMap::new();
        let mut recipes = Vec::with_capacity(documents.components.len());
        for definition in documents.components {
            let (component, profile_versions) =
                resolve_recipe_component(definition, &documents.profiles, &mut profile_cache)?;
            validate_recipe_component_identity(&component.id)?;
            if !ids.insert(component.id.clone()) {
                return Err(Error::InvalidInput(format!(
                    "environment specification declares component `{}` more than once",
                    component.id
                )));
            }
            if component.adapter.as_str() != RECIPE_ADAPTER_IDENTITY {
                return Err(Error::InvalidInput(format!(
                    "component `{}` uses unsupported declarative adapter `{}`; this specification host currently accepts only `{RECIPE_ADAPTER_IDENTITY}`",
                    component.id, component.adapter
                )));
            }
            if component.kind != "generated" {
                return Err(Error::InvalidInput(format!(
                    "command component `{}` must use kind = \"generated\"",
                    component.id
                )));
            }
            if component.outputs.is_empty() || component.outputs.len() > 32 {
                return Err(Error::InvalidInput(format!(
                    "command component `{}` must declare between 1 and 32 outputs",
                    component.id,
                )));
            }
            let mut output_names = BTreeSet::new();
            for (index, output) in component.outputs.iter().enumerate() {
                let name = output
                    .name
                    .clone()
                    .unwrap_or_else(|| format!("output-{index}"));
                validate_recipe_output_name(&name, &component.id)?;
                if !output_names.insert(name.clone()) {
                    return Err(Error::InvalidInput(format!(
                        "command component `{}` declares output name `{name}` more than once",
                        component.id
                    )));
                }
                let target = normalize_relative_path(&output.target)?;
                if let Some((other_target, other_id)) = recipe_target_overlap(&targets, &target) {
                    return Err(Error::InvalidInput(format!(
                        "command component `{}` target `{target}` overlaps component `{other_id}` target `{other_target}`",
                        component.id
                    )));
                }
                targets.insert(target, format!("{}:{name}", component.id));
            }
            let schema = documents.schema.ok_or_else(|| {
                Error::Corrupt("environment specification graph lost its schema version".into())
            })?;
            if schema == RecipeSchemaVersion::V2 {
                validate_recipe_v2_component(&component, &documents.defaults)?;
            }
            let canonical = serde_json::to_vec(&(schema.as_str(), &component, &profile_versions))?;
            recipes.push(CommandRecipe {
                schema,
                specification_digest: sha256_hex(&canonical),
                specification_sources: documents.specification_sources.clone(),
                profile_versions,
                defaults: documents.defaults.clone(),
                component,
            });
        }
        recipes.sort_by(|left, right| left.component.id.cmp(&right.component.id));
        Ok(recipes)
    }

    #[allow(clippy::too_many_arguments)]
    fn collect_recipe_document(
        &self,
        source_root: &ObjectId,
        path: &str,
        depth: usize,
        is_root: bool,
        documents: &mut RecipeDocuments,
        visited: &mut BTreeSet<String>,
        stack: &mut Vec<String>,
        total_bytes: &mut u64,
    ) -> Result<()> {
        if depth > MAX_RECIPE_INCLUDE_DEPTH {
            return Err(Error::InvalidInput(format!(
                "environment specification include depth exceeds {MAX_RECIPE_INCLUDE_DEPTH} at `{path}`"
            )));
        }
        if let Some(index) = stack.iter().position(|candidate| candidate == path) {
            let mut cycle = stack[index..].to_vec();
            cycle.push(path.to_string());
            return Err(Error::InvalidInput(format!(
                "environment specification include cycle: {}",
                cycle.join(" -> ")
            )));
        }
        if visited.contains(path) {
            return Ok(());
        }
        if visited.len().saturating_add(stack.len()) >= MAX_RECIPE_INCLUDE_FILES {
            return Err(Error::InvalidInput(format!(
                "environment specification includes more than {MAX_RECIPE_INCLUDE_FILES} files"
            )));
        }
        let entry = self.root_file_entry(source_root, path)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "environment specification include `{path}` does not exist in the pinned source root"
            ))
        })?;
        if entry.size_bytes > MAX_RECIPE_SPEC_BYTES {
            return Err(Error::InvalidInput(format!(
                "environment specification `{path}` is {} bytes; the per-file maximum is {MAX_RECIPE_SPEC_BYTES}",
                entry.size_bytes
            )));
        }
        *total_bytes = total_bytes.checked_add(entry.size_bytes).ok_or_else(|| {
            Error::InvalidInput("environment specification size overflowed".to_string())
        })?;
        if *total_bytes > MAX_RECIPE_TOTAL_SPEC_BYTES {
            return Err(Error::InvalidInput(format!(
                "environment specifications total more than {MAX_RECIPE_TOTAL_SPEC_BYTES} bytes"
            )));
        }
        let entries = BTreeMap::from([(path.to_string(), entry.clone())]);
        let bytes = self
            .materialize_entries_bytes(&entries)?
            .remove(path)
            .ok_or_else(|| Error::Corrupt(format!("failed to read `{path}` from source root")))?;
        let text = String::from_utf8(bytes).map_err(|_| {
            Error::InvalidInput(format!("environment specification `{path}` must be UTF-8"))
        })?;
        let specification: RecipeSpecification = toml::from_str(&text).map_err(|err| {
            Error::InvalidInput(format!("invalid environment specification `{path}`: {err}"))
        })?;
        let schema = validate_recipe_specification_header(&specification, path)?;
        if let Some(expected) = documents.schema {
            if schema != expected {
                return Err(Error::InvalidInput(format!(
                    "environment specification `{path}` uses schema `{}` but the root document uses `{}`",
                    schema.as_str(),
                    expected.as_str()
                )));
            }
        } else {
            documents.schema = Some(schema);
        }

        stack.push(path.to_string());
        for include in &specification.include {
            let include_path = resolve_recipe_include_path(path, include)?;
            self.collect_recipe_document(
                source_root,
                &include_path,
                depth + 1,
                false,
                documents,
                visited,
                stack,
                total_bytes,
            )?;
        }
        stack.pop();

        for (name, profile) in specification.profile {
            let canonical_name = canonical_recipe_profile_name(&name)?;
            if documents
                .profiles
                .insert(canonical_name.clone(), profile)
                .is_some()
            {
                return Err(Error::InvalidInput(format!(
                    "environment specifications declare profile `{canonical_name}` more than once"
                )));
            }
        }
        documents.components.extend(specification.components);
        if is_root {
            documents.defaults = specification.environment;
        }
        documents
            .specification_sources
            .insert(path.to_string(), entry.content_hash);
        visited.insert(path.to_string());
        Ok(())
    }

    fn plan_command_recipe(
        &self,
        source_root: &ObjectId,
        recipe: CommandRecipe,
    ) -> Result<WorkspaceEnvironmentPlan> {
        self.plan_command_recipe_with_tool(source_root, recipe, None)
    }

    fn plan_command_recipe_with_tool(
        &self,
        source_root: &ObjectId,
        recipe: CommandRecipe,
        resolved_tool: Option<ResolvedWorkspaceTool>,
    ) -> Result<WorkspaceEnvironmentPlan> {
        let component = recipe.component;
        let network = component
            .build
            .network
            .as_deref()
            .unwrap_or(&recipe.defaults.default_network);
        let scripts = component
            .build
            .scripts
            .as_deref()
            .unwrap_or(&recipe.defaults.default_scripts);
        if network != "deny" || scripts != "deny" {
            return Err(Error::InvalidInput(format!(
                "command component `{}` requires network = \"deny\" and scripts = \"deny\"",
                component.id
            )));
        }
        if component.build.command.is_empty() {
            return Err(Error::InvalidInput(format!(
                "command component `{}` has an empty build.command",
                component.id
            )));
        }
        if component.build.command.len() > 4096
            || component.build.command.iter().any(|argument| {
                argument.len() > 128 * 1024
                    || argument.contains('\0')
                    || contains_sensitive_text(argument)
            })
        {
            return Err(Error::InvalidInput(format!(
                "command component `{}` exceeds command argument limits",
                component.id
            )));
        }
        let program = &component.build.command[0];
        if program.contains('/') || program.contains('\\') || is_shell_program(program) {
            return Err(Error::InvalidInput(format!(
                "command component `{}` must name a non-shell executable from PATH, not `{program}`",
                component.id
            )));
        }
        let tool = resolved_tool
            .map(Ok)
            .unwrap_or_else(|| resolve_workspace_tool_executable(program))?;
        validate_recipe_tool_path(self, &tool.path, &component.id)?;
        let cwd = normalize_recipe_path_allow_root(
            component.build.cwd.as_deref().unwrap_or(&component.root),
        )?;
        if !component.root.is_empty()
            && cwd != component.root
            && !cwd.starts_with(&format!("{}/", component.root))
        {
            return Err(Error::InvalidInput(format!(
                "command component `{}` cwd `{}` escapes its root `{}`",
                component.id, cwd, component.root
            )));
        }
        let selected_inputs = self.expand_recipe_inputs(source_root, &component)?;
        let mut outputs = Vec::with_capacity(component.outputs.len());
        let mut output_paths = Vec::<(String, String)>::new();
        let mut portability = None;
        for (index, output) in component.outputs.iter().enumerate() {
            let policy = output.policy;
            let (default_reuse, default_scope, default_publish) = if policy.has_immutable_layer() {
                (
                    EnvironmentReuseMode::Exact,
                    EnvironmentSharingScope::Workspace,
                    EnvironmentPublicationTrigger::OnSync,
                )
            } else {
                (
                    EnvironmentReuseMode::None,
                    EnvironmentSharingScope::Lane,
                    EnvironmentPublicationTrigger::Never,
                )
            };
            let reuse = output.reuse.unwrap_or(default_reuse);
            let scope = output.scope.unwrap_or(default_scope);
            let publish = output.publish.unwrap_or(default_publish);
            validate_environment_output_contract(
                policy,
                reuse,
                scope,
                publish,
                output.gate.as_deref(),
                false,
            )?;
            if output.portability != "host" && output.portability != "platform" {
                return Err(Error::InvalidInput(format!(
                    "command component `{}` output portability must be `host` or `platform`",
                    component.id
                )));
            }
            if portability
                .as_deref()
                .is_some_and(|value| value != output.portability)
            {
                return Err(Error::InvalidInput(format!(
                    "command component `{}` outputs must currently use one portability class",
                    component.id
                )));
            }
            portability = Some(output.portability.clone());
            let name = output
                .name
                .clone()
                .unwrap_or_else(|| format!("output-{index}"));
            validate_recipe_output_name(&name, &component.id)?;
            let output_source = normalize_relative_path(&output.source)?;
            let output_repository_path = join_recipe_path(&cwd, &output_source);
            for (other_name, other_path) in &output_paths {
                if recipe_paths_overlap(&output_repository_path, other_path) {
                    return Err(Error::InvalidInput(format!(
                        "command component `{}` output `{name}` path `{output_repository_path}` overlaps output `{other_name}` path `{other_path}`",
                        component.id
                    )));
                }
            }
            for path in selected_inputs.keys() {
                if recipe_paths_overlap(path, &output_repository_path) {
                    return Err(Error::InvalidInput(format!(
                        "command component `{}` output `{output_repository_path}` overlaps declared input `{path}`",
                        component.id
                    )));
                }
            }
            let mount_path = normalize_relative_path(&output.target)?;
            output_paths.push((name.clone(), output_repository_path.clone()));
            outputs.push(WorkspaceEnvironmentOutput {
                name,
                output_path: format!("project/{output_repository_path}"),
                mount_path,
                policy,
                reuse,
                scope,
                publish,
                gate: output.gate.clone(),
                create_if_missing: true,
            });
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
                        output.reuse.as_str(),
                        output.scope.as_str(),
                        output.publish.as_str(),
                        &output.gate,
                    )
                })
                .collect::<Vec<_>>(),
        )?;
        let mut layer_inputs = BTreeMap::from([
            (
                "specification_digest".to_string(),
                recipe.specification_digest.clone(),
            ),
            ("component_id".to_string(), component.id.clone()),
            ("component_root".to_string(), component.root.clone()),
            (
                "command".to_string(),
                serde_json::to_string(&component.build.command)?,
            ),
            ("cwd".to_string(), cwd.clone()),
            ("output_contract".to_string(), output_contract),
            ("network".to_string(), "deny".to_string()),
            ("scripts".to_string(), "deny".to_string()),
            (
                "capability_contract".to_string(),
                "fs-read:declared-inputs;fs-write:declared-outputs+isolated-home+tmp;process:exact-executable;child-exec:deny;network:deny;shell:deny;scripts:deny;secrets:deny"
                    .to_string(),
            ),
            (
                "adapter_implementation".to_string(),
                env!("CARGO_PKG_VERSION").to_string(),
            ),
            (
                "adapter_distribution_digest".to_string(),
                "builtin:command-recipe-plan-v1".to_string(),
            ),
        ]);
        for (path, digest) in &recipe.specification_sources {
            layer_inputs.insert(format!("specification_source:{path}"), digest.clone());
        }
        for (profile, version) in &recipe.profile_versions {
            layer_inputs.insert(format!("profile:{profile}"), version.clone());
        }
        for (path, entry) in &selected_inputs {
            layer_inputs.insert(format!("input:{path}"), entry.content_hash.clone());
        }
        for (name, value) in &component.build.environment {
            validate_recipe_environment(name, value, &component.id)?;
            layer_inputs.insert(format!("environment:{name}"), value.clone());
        }
        let inputs = selected_inputs
            .into_iter()
            .map(|(path, entry)| WorkspaceEnvironmentInput {
                source_path: path.clone(),
                staging_path: format!("project/{path}"),
                entry,
            })
            .collect::<Vec<_>>();
        let portability_scope = if portability.as_deref() == Some("platform") {
            "recipe-tool-platform"
        } else {
            "recipe-tool-host"
        };
        Ok(WorkspaceEnvironmentPlan {
            component_id: component.id,
            adapter_identity: RECIPE_ADAPTER_IDENTITY.to_string(),
            adapter_version: 1,
            implementation_version: env!("CARGO_PKG_VERSION").to_string(),
            distribution_digest: "builtin:command-recipe-plan-v1".to_string(),
            kind: "generated".to_string(),
            dependencies: component
                .dependencies
                .into_iter()
                .map(|dependency| {
                    Ok(WorkspaceEnvironmentDependency::build_requires(dependency))
                })
                .chain(component.edges.into_iter().map(|edge| {
                    Ok(WorkspaceEnvironmentDependency {
                        component_id: edge.component,
                        edge_type: WorkspaceEnvironmentEdgeType::parse(&edge.edge_type)?,
                    })
                }))
                .collect::<Result<Vec<_>>>()?,
            resolved_dependencies: Vec::new(),
            layer_key: WorkspaceLayerKeyV1 {
                kind: "generated".to_string(),
                adapter: "command".to_string(),
                adapter_version: 1,
                inputs: layer_inputs,
                tool_versions: BTreeMap::from([(
                    format!("executable:{program}"),
                    tool.identity.clone(),
                )]),
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: portability_scope.to_string(),
                strategy: "restricted-command-recipe-v1".to_string(),
            },
            inputs,
            resolution_inputs: Vec::new(),
            source_projection: None,
            pre_commands: Vec::new(),
            command: Some(WorkspaceEnvironmentCommand {
                program: program.clone(),
                resolved_program: tool.path,
                executable_identity: tool.identity,
                args: component.build.command.into_iter().skip(1).collect(),
                working_directory: if cwd.is_empty() {
                    "project".to_string()
                } else {
                    format!("project/{cwd}")
                },
                environment: component.build.environment,
                remove_environment: Vec::new(),
                cache_names: Vec::new(),
            }),
            mounted_commands: Vec::new(),
            caches: Vec::new(),
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe,
            outputs,
            stale_reason:
                "environment specification, declared input, executable, platform, or command policy changed"
                    .to_string(),
        })
    }

    fn expand_recipe_inputs(
        &self,
        source_root: &ObjectId,
        component: &RecipeComponent,
    ) -> Result<BTreeMap<String, FileEntry>> {
        if component.inputs.is_empty() {
            return Err(Error::InvalidInput(format!(
                "command component `{}` must declare at least one identity input",
                component.id
            )));
        }
        let mut exact = Vec::new();
        let mut patterns = Vec::new();
        for input in &component.inputs {
            if input.role != "identity" || input.format != "bytes" {
                return Err(Error::InvalidInput(format!(
                    "command component `{}` currently accepts only role = \"identity\", format = \"bytes\" inputs",
                    component.id
                )));
            }
            validate_recipe_pattern(&input.path)?;
            if contains_glob_meta(&input.path) {
                patterns.push(input);
            } else {
                exact.push(input);
            }
        }
        let exact_paths = exact
            .iter()
            .map(|input| normalize_relative_path(&input.path))
            .collect::<Result<Vec<_>>>()?;
        let mut selected = self.load_root_files_for_selections(source_root, &exact_paths)?;
        for input in exact {
            let normalized = normalize_relative_path(&input.path)?;
            let matched = selected
                .keys()
                .any(|path| path == &normalized || path.starts_with(&format!("{normalized}/")));
            if !matched && !input.optional {
                return Err(Error::InvalidInput(format!(
                    "command component `{}` required input `{}` did not match a file or directory",
                    component.id, input.path
                )));
            }
        }
        if !patterns.is_empty() {
            let mut builder = GlobSetBuilder::new();
            for input in &patterns {
                builder.add(
                    GlobBuilder::new(&input.path)
                        .literal_separator(true)
                        .backslash_escape(false)
                        .build()
                        .map_err(|err| {
                            Error::InvalidInput(format!(
                                "command component `{}` has invalid input glob `{}`: {err}",
                                component.id, input.path
                            ))
                        })?,
                );
            }
            let matcher = builder.build().map_err(|err| {
                Error::InvalidInput(format!(
                    "command component `{}` input glob set is invalid: {err}",
                    component.id
                ))
            })?;
            let mut matched_counts = vec![0usize; patterns.len()];
            self.for_each_root_file_chunk(source_root, 1024, |chunk| {
                for (path, entry) in chunk {
                    let matches = matcher.matches(&path);
                    if matches.is_empty() {
                        continue;
                    }
                    for index in matches {
                        matched_counts[index] += 1;
                    }
                    selected.insert(path, entry);
                }
                Ok(())
            })?;
            for (input, count) in patterns.iter().zip(matched_counts) {
                if count == 0 && !input.optional {
                    return Err(Error::InvalidInput(format!(
                        "command component `{}` required input glob `{}` matched no files",
                        component.id, input.path
                    )));
                }
            }
        }
        let total_bytes = selected.values().try_fold(0u64, |total, entry| {
            total.checked_add(entry.size_bytes).ok_or_else(|| {
                Error::InvalidInput(format!(
                    "command component `{}` input byte count overflowed",
                    component.id
                ))
            })
        })?;
        if selected.len() > MAX_RECIPE_INPUT_FILES || total_bytes > MAX_RECIPE_INPUT_BYTES {
            return Err(Error::InvalidInput(format!(
                "command component `{}` selects {} files and {total_bytes} bytes; limits are {MAX_RECIPE_INPUT_FILES} files and {MAX_RECIPE_INPUT_BYTES} bytes",
                component.id,
                selected.len()
            )));
        }
        Ok(selected)
    }
}

fn validate_recipe_specification_header(
    specification: &RecipeSpecification,
    path: &str,
) -> Result<RecipeSchemaVersion> {
    let schema = RecipeSchemaVersion::parse(&specification.schema, path)?;
    if schema == RecipeSchemaVersion::V1
        && (specification.environment.missing_resolution.is_some()
            || specification.profile.values().any(recipe_profile_uses_v2)
            || specification
                .components
                .iter()
                .any(recipe_component_definition_uses_v2))
    {
        return Err(Error::InvalidInput(format!(
            "environment specification `{path}` uses fields that require `{RECIPE_SCHEMA_V2}`"
        )));
    }
    if specification.environment.default_network != "deny"
        || specification.environment.default_scripts != "deny"
    {
        return Err(Error::InvalidInput(format!(
            "environment specification `{path}` must set default_network and default_scripts to `deny`"
        )));
    }
    let _environment_name = specification.environment.name.as_deref();
    Ok(schema)
}

fn recipe_profile_uses_v2(profile: &RecipeProfile) -> bool {
    profile.resolution.is_some()
        || !profile.actions.is_empty()
        || !profile.validations.is_empty()
        || profile.capabilities.is_some()
        || !profile.source_exports.is_empty()
}

fn recipe_component_definition_uses_v2(component: &RecipeComponentDefinition) -> bool {
    component.resolution.is_some()
        || !component.actions.is_empty()
        || !component.validations.is_empty()
        || component.capabilities.is_some()
        || !component.source_exports.is_empty()
}

fn resolve_recipe_include_path(including_path: &str, include: &str) -> Result<String> {
    if include.is_empty()
        || include.starts_with('/')
        || include.contains("://")
        || include.contains('\\')
        || contains_glob_meta(include)
        || include
            .split('/')
            .any(|segment| segment.is_empty() || segment == "." || segment == "..")
    {
        return Err(Error::InvalidInput(format!(
            "invalid local environment specification include `{include}` in `{including_path}`"
        )));
    }
    let parent = including_path
        .rsplit_once('/')
        .map(|(parent, _)| parent)
        .unwrap_or("");
    normalize_relative_path(&join_recipe_path(parent, include))
}

fn validate_recipe_profile_name(name: &str) -> Result<()> {
    let canonical = name.strip_prefix("profile.").unwrap_or(name);
    if canonical.is_empty()
        || canonical.len() > 256
        || !canonical
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | '/'))
        || canonical
            .split('/')
            .any(|segment| segment.is_empty() || segment == "..")
    {
        return Err(Error::InvalidInput(format!(
            "invalid environment recipe profile name `{name}`"
        )));
    }
    Ok(())
}

fn canonical_recipe_profile_name(name: &str) -> Result<String> {
    validate_recipe_profile_name(name)?;
    Ok(name.strip_prefix("profile.").unwrap_or(name).to_string())
}

fn recipe_profile_fragment(profile: &RecipeProfile) -> RecipeFragment {
    RecipeFragment {
        adapter: profile.adapter.clone(),
        kind: profile.kind.clone(),
        dependencies: profile.depends_on.clone(),
        edges: profile.edges.clone(),
        inputs: profile.inputs.clone(),
        outputs: profile.outputs.clone(),
        build: profile.build.clone(),
        resolution: profile.resolution.clone(),
        actions: profile.actions.clone(),
        validations: profile.validations.clone(),
        capabilities: profile.capabilities.clone(),
        source_exports: profile.source_exports.clone(),
    }
}

fn apply_recipe_fragment(target: &mut RecipeFragment, source: &RecipeFragment) {
    if source.adapter.is_some() {
        target.adapter.clone_from(&source.adapter);
    }
    if source.kind.is_some() {
        target.kind.clone_from(&source.kind);
    }
    target.dependencies.extend(source.dependencies.clone());
    target.edges.extend(source.edges.clone());
    target.inputs.extend(source.inputs.clone());
    if !source.outputs.is_empty() {
        target.outputs.clone_from(&source.outputs);
    }
    if source.build.is_some() {
        target.build.clone_from(&source.build);
    }
    if source.resolution.is_some() {
        target.resolution.clone_from(&source.resolution);
    }
    target.actions.extend(source.actions.clone());
    target.validations.extend(source.validations.clone());
    if source.capabilities.is_some() {
        target.capabilities.clone_from(&source.capabilities);
    }
    target.source_exports.extend(source.source_exports.clone());
}

fn resolve_recipe_profile(
    requested_name: &str,
    profiles: &BTreeMap<String, RecipeProfile>,
    cache: &mut BTreeMap<String, ResolvedRecipeProfile>,
    stack: &mut Vec<String>,
) -> Result<ResolvedRecipeProfile> {
    let name = canonical_recipe_profile_name(requested_name)?;
    if let Some(resolved) = cache.get(&name) {
        return Ok(resolved.clone());
    }
    if let Some(index) = stack.iter().position(|candidate| candidate == &name) {
        let mut cycle = stack[index..].to_vec();
        cycle.push(name);
        return Err(Error::InvalidInput(format!(
            "environment recipe profile cycle: {}",
            cycle.join(" -> ")
        )));
    }
    let profile = profiles.get(&name).ok_or_else(|| {
        Error::InvalidInput(format!(
            "environment recipe references unknown profile `{requested_name}`"
        ))
    })?;
    if profile.version.is_empty()
        || profile.version.len() > 128
        || profile.version.contains(char::is_whitespace)
        || profile.version.contains('\0')
    {
        return Err(Error::InvalidInput(format!(
            "environment recipe profile `{name}` has invalid version `{}`",
            profile.version
        )));
    }

    stack.push(name.clone());
    let mut fragment = RecipeFragment::default();
    let mut versions = BTreeMap::new();
    for parent in &profile.extends {
        let resolved = resolve_recipe_profile(parent, profiles, cache, stack)?;
        apply_recipe_fragment(&mut fragment, &resolved.fragment);
        versions.extend(resolved.versions);
    }
    stack.pop();
    apply_recipe_fragment(&mut fragment, &recipe_profile_fragment(profile));
    versions.insert(name.clone(), profile.version.clone());
    let resolved = ResolvedRecipeProfile { fragment, versions };
    cache.insert(name, resolved.clone());
    Ok(resolved)
}

fn resolve_recipe_component(
    definition: RecipeComponentDefinition,
    profiles: &BTreeMap<String, RecipeProfile>,
    cache: &mut BTreeMap<String, ResolvedRecipeProfile>,
) -> Result<(RecipeComponent, BTreeMap<String, String>)> {
    validate_recipe_component_identity(&definition.id)?;
    let root = normalize_recipe_path_allow_root(&definition.root)?;
    let mut fragment = RecipeFragment::default();
    let mut versions = BTreeMap::new();
    let mut stack = Vec::new();
    for profile_name in &definition.extends {
        let resolved = resolve_recipe_profile(profile_name, profiles, cache, &mut stack)?;
        apply_recipe_fragment(&mut fragment, &resolved.fragment);
        versions.extend(resolved.versions);
    }
    apply_recipe_fragment(
        &mut fragment,
        &RecipeFragment {
            adapter: definition.adapter,
            kind: definition.kind,
            dependencies: definition.depends_on,
            edges: definition.edges,
            inputs: definition.inputs,
            outputs: definition.outputs,
            build: definition.build,
            resolution: definition.resolution,
            actions: definition.actions,
            validations: definition.validations,
            capabilities: definition.capabilities,
            source_exports: definition.source_exports,
        },
    );
    let adapter = fragment.adapter.ok_or_else(|| {
        Error::InvalidInput(format!(
            "command component `{}` has no adapter after profile expansion",
            definition.id
        ))
    })?;
    let mut build = fragment.build.ok_or_else(|| {
        Error::InvalidInput(format!(
            "command component `{}` has no build declaration after profile expansion",
            definition.id
        ))
    })?;
    let mut inputs = fragment.inputs;
    let mut outputs = fragment.outputs;
    let mut resolution = fragment.resolution;
    let mut actions = fragment.actions;
    let mut validations = fragment.validations;
    let capabilities = fragment.capabilities;
    let mut source_exports = fragment.source_exports;
    let mut dependencies = fragment.dependencies;
    let edges = fragment.edges;
    let mut seen_dependencies = BTreeSet::new();
    dependencies.retain(|dependency| seen_dependencies.insert(dependency.clone()));
    for dependency in &dependencies {
        validate_recipe_component_identity(dependency)?;
        if dependency == &definition.id {
            return Err(Error::InvalidInput(format!(
                "environment component `{}` cannot depend on itself",
                definition.id
            )));
        }
    }
    let mut typed_edge_components = BTreeMap::new();
    for edge in edges {
        if let Some(previous) =
            typed_edge_components.insert(edge.component.clone(), edge.edge_type.clone())
            && previous != edge.edge_type
        {
            return Err(Error::InvalidInput(format!(
                    "environment component `{}` declares conflicting edge types `{previous}` and `{}` for `{}`",
                    definition.id, edge.edge_type, edge.component
                )));
        }
    }
    let edges = typed_edge_components
        .into_iter()
        .map(|(component, edge_type)| RecipeDependencyEdge {
            component,
            edge_type,
        })
        .collect::<Vec<_>>();
    for edge in &edges {
        validate_recipe_component_identity(&edge.component)?;
        WorkspaceEnvironmentEdgeType::parse(&edge.edge_type)?;
        if edge.component == definition.id {
            return Err(Error::InvalidInput(format!(
                "environment component `{}` cannot depend on itself",
                definition.id
            )));
        }
        if seen_dependencies.contains(&edge.component) {
            return Err(Error::InvalidInput(format!(
                "environment component `{}` declares both legacy depends_on and typed edge for `{}`",
                definition.id, edge.component
            )));
        }
    }
    for input in &mut inputs {
        input.path = expand_recipe_root_template(&input.path, &root);
    }
    for output in &mut outputs {
        output.source = expand_recipe_root_template(&output.source, &root);
        output.target = expand_recipe_root_template(&output.target, &root);
    }
    for argument in &mut build.command {
        *argument = expand_recipe_root_template(argument, &root);
    }
    if let Some(cwd) = &mut build.cwd {
        *cwd = expand_recipe_root_template(cwd, &root);
    }
    for value in build.environment.values_mut() {
        *value = expand_recipe_root_template(value, &root);
    }
    if let Some(resolution) = &mut resolution {
        for argument in &mut resolution.command {
            *argument = expand_recipe_root_template(argument, &root);
        }
        if let Some(cwd) = &mut resolution.cwd {
            *cwd = expand_recipe_root_template(cwd, &root);
        }
        resolution.snapshot = expand_recipe_root_template(&resolution.snapshot, &root);
        for value in resolution.environment.values_mut() {
            *value = expand_recipe_root_template(value, &root);
        }
    }
    for action in &mut actions {
        for argument in &mut action.command {
            *argument = expand_recipe_root_template(argument, &root);
        }
        if let Some(cwd) = &mut action.cwd {
            *cwd = expand_recipe_root_template(cwd, &root);
        }
        for value in action.environment.values_mut() {
            *value = expand_recipe_root_template(value, &root);
        }
    }
    for validation in &mut validations {
        if let Some(path) = &mut validation.path {
            *path = expand_recipe_root_template(path, &root);
        }
        for argument in &mut validation.command {
            *argument = expand_recipe_root_template(argument, &root);
        }
    }
    for export in &mut source_exports {
        export.source = expand_recipe_root_template(&export.source, &root);
        export.target = expand_recipe_root_template(&export.target, &root);
    }
    let mut seen_inputs = BTreeSet::new();
    inputs.retain(|input| {
        seen_inputs.insert((
            input.path.clone(),
            input.role.clone(),
            input.format.clone(),
            input.optional,
        ))
    });
    Ok((
        RecipeComponent {
            id: definition.id,
            adapter,
            root,
            kind: fragment.kind.unwrap_or_else(default_recipe_kind),
            dependencies,
            edges,
            inputs,
            outputs,
            build,
            resolution,
            actions,
            validations,
            capabilities,
            source_exports,
        },
        versions,
    ))
}

fn expand_recipe_root_template(value: &str, root: &str) -> String {
    if root.is_empty() {
        value.replace("{root}/", "").replace("{root}", ".")
    } else {
        value.replace("{root}", root)
    }
}

fn validate_recipe_component_identity(id: &str) -> Result<()> {
    if id.is_empty()
        || id.len() > 256
        || !id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_' | ':' | '/'))
        || id.starts_with('/')
        || id.ends_with('/')
        || id
            .split('/')
            .any(|segment| segment.is_empty() || segment == "..")
    {
        return Err(Error::InvalidInput(format!(
            "invalid command component id `{id}`"
        )));
    }
    Ok(())
}

fn validate_recipe_output_name(name: &str, component_id: &str) -> Result<()> {
    if name.is_empty()
        || name.len() > 128
        || !name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_'))
    {
        return Err(Error::InvalidInput(format!(
            "command component `{component_id}` has invalid output name `{name}`"
        )));
    }
    Ok(())
}

fn validate_recipe_pattern(pattern: &str) -> Result<()> {
    if pattern.is_empty()
        || pattern.starts_with('/')
        || pattern.contains('\\')
        || pattern.split('/').any(|segment| segment == "..")
    {
        return Err(Error::InvalidInput(format!(
            "invalid repository-relative recipe input `{pattern}`"
        )));
    }
    normalize_relative_path(pattern).map(|_| ())
}

fn contains_glob_meta(path: &str) -> bool {
    path.bytes()
        .any(|byte| matches!(byte, b'*' | b'?' | b'[' | b'{' | b'!'))
}

pub(super) fn validate_recipe_environment(
    name: &str,
    value: &str,
    component_id: &str,
) -> Result<()> {
    let valid_name = !name.is_empty()
        && name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric());
    let upper = name.to_ascii_uppercase();
    let sensitive = [
        "TOKEN",
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "CREDENTIAL",
        "PRIVATE_KEY",
        "AUTH",
    ]
    .iter()
    .any(|needle| upper.contains(needle));
    if !valid_name
        || sensitive
        || matches!(
            upper.as_str(),
            "PATH" | "HOME" | "TMP" | "TMPDIR" | "TEMP" | "SHELL" | "DYLD_INSERT_LIBRARIES"
        )
        || value.contains('\0')
        || value.len() > 128 * 1024
        || contains_sensitive_text(value)
    {
        return Err(Error::InvalidInput(format!(
            "command component `{component_id}` has forbidden environment entry `{name}`"
        )));
    }
    Ok(())
}

pub(super) fn validate_recipe_tool_path(db: &Trail, path: &Path, component_id: &str) -> Result<()> {
    let canonical = fs::canonicalize(path)?;
    let mut forbidden = vec![db.workspace_root.clone(), db.db_dir.clone()];
    if let Some(home) = std::env::var_os("HOME") {
        forbidden.push(PathBuf::from(home));
    }
    if forbidden.iter().any(|root| canonical.starts_with(root)) {
        return Err(Error::InvalidInput(format!(
            "command component `{component_id}` executable `{}` is under a mutable workspace or user home; bind a host-managed toolchain instead",
            canonical.display()
        )));
    }
    Ok(())
}

pub(super) fn is_shell_program(program: &str) -> bool {
    matches!(
        program.to_ascii_lowercase().as_str(),
        "sh" | "bash"
            | "zsh"
            | "fish"
            | "dash"
            | "ksh"
            | "csh"
            | "tcsh"
            | "cmd"
            | "cmd.exe"
            | "powershell"
            | "powershell.exe"
            | "pwsh"
    )
}

fn normalize_recipe_path_allow_root(path: &str) -> Result<String> {
    if path.trim_matches('/').is_empty() || path == "." {
        Ok(String::new())
    } else {
        normalize_relative_path(path)
    }
}

fn join_recipe_path(root: &str, child: &str) -> String {
    if root.is_empty() {
        child.to_string()
    } else {
        format!("{root}/{child}")
    }
}

fn recipe_paths_overlap(left: &str, right: &str) -> bool {
    left == right
        || left.starts_with(&format!("{right}/"))
        || right.starts_with(&format!("{left}/"))
}

fn recipe_target_overlap<'a>(
    targets: &'a BTreeMap<String, String>,
    target: &str,
) -> Option<(&'a str, &'a str)> {
    if let Some((stored, owner)) = targets.get_key_value(target) {
        return Some((stored, owner));
    }
    let mut prefix = String::new();
    let mut segments = target.split('/').peekable();
    while let Some(segment) = segments.next() {
        if !prefix.is_empty() {
            prefix.push('/');
        }
        prefix.push_str(segment);
        if segments.peek().is_some()
            && let Some((stored, owner)) = targets.get_key_value(&prefix)
        {
            return Some((stored, owner));
        }
    }
    targets
        .range(target.to_string()..)
        .next()
        .filter(|(stored, _)| stored.starts_with(&format!("{target}/")))
        .map(|(stored, owner)| (stored.as_str(), owner.as_str()))
}

fn display_recipe_root(root: &str) -> &str {
    if root.is_empty() {
        "."
    } else {
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ordered_recipe_target_overlap_finds_ancestors_and_descendants() {
        let descendants = BTreeMap::from([("generated/nested".to_string(), "child".to_string())]);
        assert_eq!(
            recipe_target_overlap(&descendants, "generated"),
            Some(("generated/nested", "child"))
        );
        let ancestors = BTreeMap::from([("generated".to_string(), "parent".to_string())]);
        assert_eq!(
            recipe_target_overlap(&ancestors, "generated/nested"),
            Some(("generated", "parent"))
        );
        assert!(recipe_target_overlap(&ancestors, "generated-sibling").is_none());
    }

    fn write_recipe_workspace(workspace: &Path, command: &[&str]) {
        write_recipe_workspace_with_policy(workspace, command, "immutable_seed_private");
    }

    fn write_recipe_workspace_with_policy(workspace: &Path, command: &[&str], policy: &str) {
        fs::write(workspace.join("input.txt"), "declared input\n").unwrap();
        let command = command
            .iter()
            .map(|value| format!("{:?}", value))
            .collect::<Vec<_>>()
            .join(", ");
        fs::write(
            workspace.join("trail.environment.toml"),
            format!(
                r#"schema = "trail.environment/v1"

[environment]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "generated.copy"
adapter = "trail/command@1"
root = "."
kind = "generated"

[[component.input]]
path = "*.txt"
role = "identity"
format = "bytes"

[component.build]
command = [{command}]
cwd = "."
network = "deny"
scripts = "deny"

[[component.output]]
name = "generated"
source = "generated"
target = ".trail-generated/copy"
policy = "{policy}"
portability = "host"
"#
            ),
        )
        .unwrap();
    }

    fn open_recipe_lane(command: &[&str]) -> (tempfile::TempDir, Trail) {
        let workspace = tempfile::tempdir().unwrap();
        write_recipe_workspace(workspace.path(), command);
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        for lane in ["recipe-a", "recipe-b"] {
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                lane,
                Some("main"),
                mode.clone(),
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
        }
        (workspace, db)
    }

    fn open_recipe_graph(specification: &str) -> (tempfile::TempDir, Trail) {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("input.txt"), "graph\n").unwrap();
        fs::write(
            workspace.path().join("trail.environment.toml"),
            specification,
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "graph",
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
        (workspace, db)
    }

    #[test]
    fn command_recipe_discovery_and_plan_are_side_effect_free_and_exact() {
        let (_workspace, db) = open_recipe_lane(&["cp", "input.txt", "generated/copied.txt"]);
        let discovery = db.discover_workspace_environment("recipe-a", None).unwrap();
        assert_eq!(discovery.components.len(), 1);
        assert_eq!(discovery.components[0].component_id, "generated.copy");
        assert_eq!(
            discovery.components[0].adapter_identity,
            RECIPE_ADAPTER_IDENTITY
        );
        let plan = db
            .command_recipe_plan(&discovery.source_root, "generated.copy")
            .unwrap();
        assert_eq!(
            plan.sandbox_policy,
            WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe
        );
        assert_eq!(plan.outputs[0].mount_path, ".trail-generated/copy");
        assert_eq!(plan.inputs.len(), 1);
        assert_eq!(plan.inputs[0].source_path, "input.txt");
        let identity = super::workspace_environment::workspace_environment_identity_contract_v3(
            &plan,
            super::workspace_environment::workspace_environment_artifact_contract_digest(&plan)
                .unwrap(),
        )
        .unwrap();
        assert!(!identity.source_closure_complete);
        assert!(!identity.portability_certified);
        assert_eq!(identity.trust_scope, "repository");
        let report = db
            .plan_workspace_environment("recipe-a", RECIPE_ADAPTER_IDENTITY, None)
            .unwrap();
        assert_eq!(report.component_id, "generated.copy");
        assert_eq!(report.capabilities.network, "deny");
        assert_eq!(report.capabilities.shell, "deny");
        assert_eq!(report.capabilities.scripts, "deny");
        assert_eq!(report.capabilities.secrets, "deny");
        assert_eq!(report.capabilities.filesystem_read, vec!["input.txt"]);
        assert_eq!(
            report.capabilities.filesystem_write,
            vec!["project/generated"]
        );
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }

    #[test]
    fn explicit_v2_schema_preserves_v1_command_recipe_planning() {
        let workspace = tempfile::tempdir().unwrap();
        write_recipe_workspace(
            workspace.path(),
            &["cp", "input.txt", "generated/copied.txt"],
        );
        let path = workspace.path().join("trail.environment.toml");
        let v1 = fs::read_to_string(&path).unwrap();
        fs::write(&path, v1.replace(RECIPE_SCHEMA_V1, RECIPE_SCHEMA_V2)).unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let source_root = db.resolve_branch_ref("main").unwrap().root_id;

        let recipes = db.load_command_recipes(&source_root).unwrap();
        assert_eq!(recipes.len(), 1);
        assert_eq!(recipes[0].component.adapter, RECIPE_ADAPTER_IDENTITY);
        let plan = db
            .command_recipe_plan(&source_root, "generated.copy")
            .unwrap();
        assert_eq!(
            plan.sandbox_policy,
            WorkspaceEnvironmentSandboxPolicy::RestrictedRecipe
        );
        assert_eq!(plan.outputs[0].mount_path, ".trail-generated/copy");
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }

    #[test]
    fn v2_schema_parses_typed_pipeline_sections_and_heterogeneous_outputs() {
        let specification = r#"schema = "trail.environment/v2"

[environment]
default_network = "deny"
default_scripts = "deny"
missing_resolution = "explicit"

[[component]]
id = "custom.pipeline"
adapter = "trail/command@1"
kind = "generated"
inputs = [{ path = "input.txt", role = "identity", format = "bytes" }]

[component.build]
command = ["cp", "input.txt", "generated/result.txt"]
cwd = "."

[component.resolve]
command = ["cp", "input.txt", "generated.lock"]
cwd = "."
network = { authorities = ["registry.example:443"] }
snapshot = "generated.lock"
format = "application/vnd.example.lock+json"

[component.resolve.capabilities]
network = "exact_authorities"
filesystem_write = "isolated_candidate"
process = "declared_executable"
child_processes = 4
secrets = "opaque_handles"
publication = "deny"

[[component.action]]
name = "construct"
phase = "construct"
command = ["cp", "input.txt", "generated/result.txt"]
cwd = "."
network = "deny"

[[component.action]]
name = "load-check"
phase = "validate"
command = ["cp", "generated/result.txt", "generated/checked.txt"]

[[component.validation]]
name = "path-contract"
kind = "path_contract"
path = "generated"
required = true
parameters = { maximum_entries = "1000" }

[component.capabilities]
network = "deny"
filesystem_read = "declared_inputs"
filesystem_write = "isolated_candidate"
process = "declared_executable"
child_processes = 1
secrets = "deny"
publication = "deny"

[[component.output]]
name = "seed"
source = "generated"
target = ".trail-generated/seed"
policy = "immutable_seed_private"
reuse = "exact"
scope = "workspace"
publish = "on_sync"

[[component.output]]
name = "scratch"
source = "scratch"
target = ".trail-generated/scratch"
policy = "disposable"
reuse = "none"
scope = "lane"
publish = "never"

[[component.source_export]]
from_output = "seed"
source = "generated-client"
target = "src/generated-client"
mode = "explicit"
collision = "fail"
validation = "path-contract"
"#;
        let (_workspace, db) = open_recipe_graph(specification);
        let source_root = db.resolve_branch_ref("main").unwrap().root_id;
        let recipes = db.load_command_recipes(&source_root).unwrap();
        let component = &recipes[0].component;

        assert_eq!(
            component.resolution.as_ref().unwrap().snapshot,
            "generated.lock"
        );
        assert_eq!(component.actions.len(), 2);
        assert_eq!(component.actions[0].phase, RecipeActionPhase::Construct);
        assert_eq!(component.actions[1].phase, RecipeActionPhase::Validate);
        assert_eq!(component.validations.len(), 1);
        assert_eq!(component.outputs.len(), 2);
        assert_eq!(
            component.outputs[0].policy,
            EnvironmentOutputPolicy::ImmutableSeedPrivate
        );
        assert_eq!(
            component.outputs[1].policy,
            EnvironmentOutputPolicy::Disposable
        );
        assert_eq!(
            component.capabilities.as_ref().unwrap().child_processes,
            Some(1)
        );
        assert_eq!(component.source_exports.len(), 1);
        assert_eq!(component.source_exports[0].target, "src/generated-client");

        let compiled = db
            .compile_repository_artifact_pipeline_v2(&source_root, "custom.pipeline")
            .unwrap();
        assert_eq!(
            compiled.proposal.status,
            EnvironmentComponentProposalStatus::Resolvable
        );
        assert_eq!(compiled.graph_plan.component_id, "custom.pipeline");
        assert_eq!(
            compiled
                .resolution_plan
                .as_ref()
                .unwrap()
                .allowed_authorities,
            vec!["registry.example:443"]
        );
        assert_eq!(compiled.desired_material.actions.len(), 4);
        assert_eq!(compiled.outputs.len(), 2);
        assert_eq!(compiled.validations.len(), 1);
        assert_eq!(compiled.source_exports.len(), 1);
        assert_eq!(
            serde_json::to_value(&compiled.source_exports[0]).unwrap(),
            serde_json::json!({
                "name": "seed",
                "output_name": "seed",
                "artifact_subpath": "generated-client",
                "destination": "src/generated-client",
                "collision_policy": "fail",
                "required_validation": "path-contract",
                "authorization_mode": "explicit"
            })
        );
        assert_eq!(
            compiled.desired_key,
            super::super::workspace_artifact::artifact_desired_key_v2(
                compiled.desired_material.clone()
            )
            .unwrap()
        );
        assert_eq!(
            compiled.desired_material.adapter_protocol,
            RECIPE_SCHEMA_V2,
            "repository v2 retains its explicit desired-key protocol instead of being relabeled as plugin v3"
        );
    }

    #[test]
    fn next_and_vite_v2_components_compose_over_node_with_private_framework_state() {
        if !Command::new("npm")
            .arg("--version")
            .output()
            .is_ok_and(|output| output.status.success())
            || !Command::new("node")
                .arg("--version")
                .output()
                .is_ok_and(|output| output.status.success())
        {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("package.json"),
            r#"{"name":"framework-composition","version":"1.0.0","private":true}"#,
        )
        .unwrap();
        fs::write(
            workspace.path().join("package-lock.json"),
            r#"{"name":"framework-composition","version":"1.0.0","lockfileVersion":3,"requires":true,"packages":{"":{"name":"framework-composition","version":"1.0.0"}}}"#,
        )
        .unwrap();
        fs::write(workspace.path().join("next-source.js"), "next fixture\n").unwrap();
        fs::write(workspace.path().join("vite-source.js"), "vite fixture\n").unwrap();
        fs::write(
            workspace.path().join("trail.environment.toml"),
            r#"schema = "trail.environment/v2"

[environment]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "web.next-build"
adapter = "trail/command@1"
kind = "generated"
depends_on = ["node"]
inputs = [{ path = "next-source.js", role = "identity", format = "bytes" }]
outputs = [{ name = "next-state", source = "next-output", target = ".next", policy = "writable_private", reuse = "none", scope = "lane", publish = "manual", portability = "host" }]
[component.build]
command = ["cp", "next-source.js", "next-output/server.js"]
cwd = "."
network = "deny"
scripts = "deny"

[[component]]
id = "web.vite-build"
adapter = "trail/command@1"
kind = "generated"
depends_on = ["node"]
inputs = [{ path = "vite-source.js", role = "identity", format = "bytes" }]
outputs = [
  { name = "dist", source = "dist", target = "dist", policy = "immutable_shared", reuse = "exact", scope = "workspace", publish = "on_sync", portability = "host" }
]
[component.build]
command = ["cp", "vite-source.js", "dist/app.js"]
cwd = "."
network = "deny"
scripts = "deny"

[[component.validation]]
name = "dist-path-contract"
kind = "path_contract"
path = "dist"
required = true
parameters = { maximum_entries = "1000" }

[[component]]
id = "web.vite-cache"
adapter = "trail/command@1"
kind = "generated"
depends_on = ["node"]
inputs = [{ path = "vite-source.js", role = "identity", format = "bytes" }]
outputs = [{ name = "vite-cache", source = "vite-cache", target = ".vite", policy = "writable_private", reuse = "none", scope = "lane", publish = "manual", portability = "host" }]
[component.build]
command = ["cp", "vite-source.js", "vite-cache/metadata.json"]
cwd = "."
network = "deny"
scripts = "deny"
"#,
        )
        .unwrap();

        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        for lane in ["framework-one", "framework-two"] {
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                lane,
                Some("main"),
                mode.clone(),
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
        }

        let graph = db
            .workspace_environment_graph("framework-one", None)
            .unwrap();
        assert_eq!(graph.nodes.len(), 4);
        assert_eq!(graph.edges.len(), 3);
        assert!(graph.edges.iter().all(|edge| {
            edge.source_component_id == "node" && edge.edge_type == "build_requires"
        }));

        let first = db
            .sync_all_workspace_environments("framework-one", None)
            .unwrap();
        let second = db
            .sync_all_workspace_environments("framework-two", None)
            .unwrap();
        assert_eq!(first.generation.components.len(), 4);
        assert_eq!(second.generation.components.len(), 4);
        let next = first
            .generation
            .components
            .iter()
            .find(|component| component.component_id == "web.next-build")
            .unwrap();
        assert_eq!(
            next.outputs[0].policy,
            EnvironmentOutputPolicy::WritablePrivate
        );
        assert!(next.outputs[0].layer_id.is_none());
        let vite = first
            .generation
            .components
            .iter()
            .find(|component| component.component_id == "web.vite-build")
            .unwrap();
        assert_eq!(vite.outputs.len(), 1);
        assert_eq!(
            vite.outputs[0].policy,
            EnvironmentOutputPolicy::ImmutableShared
        );
        assert!(vite.outputs[0].layer_id.is_some());
        let vite_cache = first
            .generation
            .components
            .iter()
            .find(|component| component.component_id == "web.vite-cache")
            .unwrap();
        assert_eq!(
            vite_cache.outputs[0].policy,
            EnvironmentOutputPolicy::WritablePrivate
        );
        assert!(vite_cache.outputs[0].layer_id.is_none());

        let first_view = db.lane_workspace_view("framework-one").unwrap().unwrap();
        let second_view = db.lane_workspace_view("framework-two").unwrap().unwrap();
        let first_generated = Path::new(&first_view.generated_upper);
        let second_generated = Path::new(&second_view.generated_upper);
        fs::write(first_generated.join(".next/lane.txt"), "one\n").unwrap();
        fs::write(second_generated.join(".next/lane.txt"), "two\n").unwrap();
        fs::write(first_generated.join(".vite/cache.txt"), "one\n").unwrap();
        fs::write(second_generated.join(".vite/cache.txt"), "two\n").unwrap();
        assert_eq!(
            fs::read_to_string(first_generated.join(".next/lane.txt")).unwrap(),
            "one\n"
        );
        assert_eq!(
            fs::read_to_string(second_generated.join(".next/lane.txt")).unwrap(),
            "two\n"
        );
        assert_eq!(
            fs::read_to_string(first_generated.join(".vite/cache.txt")).unwrap(),
            "one\n"
        );
        assert_eq!(
            fs::read_to_string(second_generated.join(".vite/cache.txt")).unwrap(),
            "two\n"
        );
    }

    #[test]
    fn v2_pipeline_sections_reject_unknown_fields_and_v1_cannot_opt_in_implicitly() {
        for (schema, extra, expected) in [
            (
                RECIPE_SCHEMA_V2,
                "[[component.action]]\nphase = \"construct\"\ncommand = [\"tool\"]\nshell = true\n",
                "unknown field",
            ),
            (
                RECIPE_SCHEMA_V1,
                "[[component.action]]\nphase = \"construct\"\ncommand = [\"tool\"]\n",
                RECIPE_SCHEMA_V2,
            ),
        ] {
            let workspace = tempfile::tempdir().unwrap();
            fs::write(workspace.path().join("input.txt"), "strict\n").unwrap();
            fs::write(
                workspace.path().join("trail.environment.toml"),
                format!(
                    "schema = {schema:?}\n[[component]]\nid = \"strict\"\nadapter = \"trail/command@1\"\ninputs = [{{ path = \"input.txt\" }}]\noutputs = [{{ source = \"generated\", target = \"generated\" }}]\n[component.build]\ncommand = [\"tool\"]\n{extra}"
                ),
            )
            .unwrap();
            Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let db = Trail::open(workspace.path()).unwrap();
            let source_root = db.resolve_branch_ref("main").unwrap().root_id;
            let error = db.load_command_recipes(&source_root).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "unexpected strict-v2 parser error: {error}"
            );
        }
    }

    #[test]
    fn v2_repository_pipeline_rejects_unsafe_authority_and_reuse_requests() {
        struct Case {
            name: &'static str,
            command: &'static [&'static str],
            before_output: &'static str,
            reuse: &'static str,
            scope: &'static str,
            after_output: &'static str,
            expected: &'static str,
        }

        let cases = [
            Case {
                name: "shell interpolation",
                command: &["cp", "$(read-secret)", "generated/result.txt"],
                before_output: "",
                reuse: "exact",
                scope: "workspace",
                after_output: "",
                expected: "unsafe or excessive argv",
            },
            Case {
                name: "shell control flow",
                command: &["cp", "input.txt", "&&", "generated/result.txt"],
                before_output: "",
                reuse: "exact",
                scope: "workspace",
                after_output: "",
                expected: "unsafe or excessive argv",
            },
            Case {
                name: "absolute host path",
                command: &["cp", "/etc/passwd", "generated/result.txt"],
                before_output: "",
                reuse: "exact",
                scope: "workspace",
                after_output: "",
                expected: "unsafe or excessive argv",
            },
            Case {
                name: "indirect child launcher",
                command: &["env", "cp", "input.txt", "generated/result.txt"],
                before_output: "",
                reuse: "exact",
                scope: "workspace",
                after_output: "",
                expected: "non-shell, non-launcher executable",
            },
            Case {
                name: "raw secret environment",
                command: &["cp", "input.txt", "generated/result.txt"],
                before_output: "[component.build.environment]\nAPI_TOKEN = \"sk-live-secret\"\n",
                reuse: "exact",
                scope: "workspace",
                after_output: "",
                expected: "forbidden environment entry",
            },
            Case {
                name: "provider socket",
                command: &["cp", "input.txt", "generated/result.txt"],
                before_output: "[component.build.environment]\nDOCKER_HOST = \"unix:///var/run/docker.sock\"\n",
                reuse: "exact",
                scope: "workspace",
                after_output: "",
                expected: "requests a provider socket",
            },
            Case {
                name: "forbidden process graph",
                command: &["cp", "input.txt", "generated/result.txt"],
                before_output: "[component.capabilities]\nprocess = \"reviewed_builtin_graph\"\n",
                reuse: "exact",
                scope: "workspace",
                after_output: "",
                expected: "exceeds the repository-declaration ceiling",
            },
            Case {
                name: "secret-tainted constructor",
                command: &["cp", "input.txt", "generated/result.txt"],
                before_output: "[component.capabilities]\nsecrets = \"opaque_handles\"\n",
                reuse: "exact",
                scope: "workspace",
                after_output: "",
                expected: "exceeds the repository-declaration ceiling",
            },
            Case {
                name: "excessive child processes",
                command: &["cp", "input.txt", "generated/result.txt"],
                before_output: "[component.capabilities]\nchild_processes = 257\n",
                reuse: "exact",
                scope: "workspace",
                after_output: "",
                expected: "child-process limit",
            },
            Case {
                name: "host-wide reuse",
                command: &["cp", "input.txt", "generated/result.txt"],
                before_output: "",
                reuse: "exact",
                scope: "host",
                after_output: "",
                expected: "host-wide artifact reuse",
            },
            Case {
                name: "compatible reuse",
                command: &["cp", "input.txt", "generated/result.txt"],
                before_output: "",
                reuse: "compatible",
                scope: "workspace",
                after_output: "",
                expected: "compatible or host-wide artifact reuse",
            },
            Case {
                name: "mounted repository action",
                command: &["cp", "input.txt", "generated/result.txt"],
                before_output: "",
                reuse: "exact",
                scope: "workspace",
                after_output: "[[component.action]]\nphase = \"mounted_execution\"\ncommand = [\"cp\", \"input.txt\", \"generated/mounted.txt\"]\n",
                expected: "requests forbidden phase",
            },
            Case {
                name: "online constructor",
                command: &["cp", "input.txt", "generated/result.txt"],
                before_output: "",
                reuse: "exact",
                scope: "workspace",
                after_output: "[[component.action]]\nphase = \"construct\"\ncommand = [\"cp\", \"input.txt\", \"generated/online.txt\"]\nnetwork = { authorities = [\"registry.example:443\"] }\n",
                expected: "must be offline",
            },
        ];

        for case in cases {
            let workspace = tempfile::tempdir().unwrap();
            fs::write(workspace.path().join("input.txt"), "strict\n").unwrap();
            let command = serde_json::to_string(case.command).unwrap();
            fs::write(
                workspace.path().join("trail.environment.toml"),
                format!(
                    r#"schema = "trail.environment/v2"

[environment]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "unsafe.pipeline"
adapter = "trail/command@1"
inputs = [{{ path = "input.txt" }}]

[component.build]
command = {command}
cwd = "."
{before_output}
[[component.output]]
name = "generated"
source = "generated"
target = ".trail-generated/unsafe"
policy = "immutable_seed_private"
reuse = "{reuse}"
scope = "{scope}"
publish = "on_sync"
{after_output}"#,
                    before_output = case.before_output,
                    reuse = case.reuse,
                    scope = case.scope,
                    after_output = case.after_output,
                ),
            )
            .unwrap();
            Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let db = Trail::open(workspace.path()).unwrap();
            let source_root = db.resolve_branch_ref("main").unwrap().root_id;
            let error = db.load_command_recipes(&source_root).unwrap_err();
            assert!(
                error.to_string().contains(case.expected),
                "{} produced unexpected error: {error}",
                case.name
            );
        }
    }

    #[test]
    fn v2_repository_authorities_are_bounded_sorted_and_deduplicated() {
        let network = RecipeNetwork::Authorities(RecipeNetworkAuthorities {
            authorities: vec![
                "registry.z.example:443".into(),
                "registry.a.example:443".into(),
                "registry.z.example:443".into(),
            ],
        });
        assert_eq!(
            recipe_network_authorities(Some(&network)).unwrap(),
            vec!["registry.a.example:443", "registry.z.example:443"]
        );

        let excessive = RecipeNetwork::Authorities(RecipeNetworkAuthorities {
            authorities: (0..=MAX_RECIPE_NETWORK_AUTHORITIES)
                .map(|index| format!("registry-{index}.example:443"))
                .collect(),
        });
        let error = recipe_network_authorities(Some(&excessive)).unwrap_err();
        assert!(error.to_string().contains("more than"));
    }

    #[test]
    fn v2_repository_input_declarations_are_bounded_and_expansion_is_sorted() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("z.txt"), "z\n").unwrap();
        fs::write(workspace.path().join("a.txt"), "a\n").unwrap();
        fs::write(
            workspace.path().join("trail.environment.toml"),
            r#"schema = "trail.environment/v2"

[[component]]
id = "sorted.inputs"
adapter = "trail/command@1"
inputs = [{ path = "*.txt" }]

[component.build]
command = ["cp", "a.txt", "generated/result.txt"]

[[component.output]]
name = "generated"
source = "generated"
target = ".trail-generated/sorted"
policy = "immutable_seed_private"
reuse = "exact"
scope = "workspace"
publish = "on_sync"
"#,
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let source_root = db.resolve_branch_ref("main").unwrap().root_id;
        let recipes = db.load_command_recipes(&source_root).unwrap();
        let selected = db
            .expand_recipe_inputs(&source_root, &recipes[0].component)
            .unwrap();
        assert_eq!(
            selected.keys().cloned().collect::<Vec<_>>(),
            vec!["a.txt", "z.txt"]
        );

        let excessive_workspace = tempfile::tempdir().unwrap();
        let inputs = (0..=MAX_RECIPE_INPUT_DECLARATIONS)
            .map(|index| format!("{{ path = \"missing/{index}.txt\", optional = true }}"))
            .collect::<Vec<_>>()
            .join(",");
        fs::write(
            excessive_workspace.path().join("trail.environment.toml"),
            format!(
                r#"schema = "trail.environment/v2"
[[component]]
id = "excessive.inputs"
adapter = "trail/command@1"
inputs = [{inputs}]
[component.build]
command = ["cp", "input.txt", "generated/result.txt"]
[[component.output]]
source = "generated"
target = ".trail-generated/excessive"
policy = "immutable_seed_private"
reuse = "exact"
scope = "workspace"
publish = "on_sync"
"#
            ),
        )
        .unwrap();
        Trail::init(
            excessive_workspace.path(),
            "main",
            InitImportMode::WorkingTree,
            false,
        )
        .unwrap();
        let excessive_db = Trail::open(excessive_workspace.path()).unwrap();
        let excessive_root = excessive_db.resolve_branch_ref("main").unwrap().root_id;
        let error = excessive_db
            .load_command_recipes(&excessive_root)
            .unwrap_err();
        assert!(error.to_string().contains("declares more than"));
    }

    #[test]
    fn repository_document_graph_rejects_mixed_schema_versions() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("config")).unwrap();
        fs::write(
            workspace.path().join("trail.environment.toml"),
            format!("schema = {RECIPE_SCHEMA_V2:?}\ninclude = [\"config/profile.toml\"]\n"),
        )
        .unwrap();
        fs::write(
            workspace.path().join("config/profile.toml"),
            format!("schema = {RECIPE_SCHEMA_V1:?}\n"),
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let source_root = db.resolve_branch_ref("main").unwrap().root_id;

        let error = db.load_command_recipes(&source_root).unwrap_err();
        assert!(error.to_string().contains("config/profile.toml"));
        assert!(error.to_string().contains(RECIPE_SCHEMA_V1));
        assert!(error.to_string().contains(RECIPE_SCHEMA_V2));
    }

    #[test]
    fn repository_document_rejects_unsupported_schema_with_supported_versions() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("trail.environment.toml"),
            "schema = \"trail.environment/v3\"\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let source_root = db.resolve_branch_ref("main").unwrap().root_id;

        let error = db.load_command_recipes(&source_root).unwrap_err();
        assert!(error.to_string().contains(RECIPE_SCHEMA_V1));
        assert!(error.to_string().contains(RECIPE_SCHEMA_V2));
    }

    #[test]
    fn command_recipe_discovery_does_not_require_or_execute_declared_tool() {
        let (_workspace, db) = open_recipe_lane(&[
            "trail-fixture-tool-that-does-not-exist",
            "input.txt",
            "generated/copied.txt",
        ]);

        let discovery = db.discover_workspace_environment("recipe-a", None).unwrap();
        assert_eq!(discovery.components.len(), 1);
        assert_eq!(discovery.components[0].component_id, "generated.copy");
        assert_eq!(
            discovery.components[0].status,
            EnvironmentComponentProposalStatus::Ready
        );
        assert!(db.list_workspace_layers().unwrap().is_empty());

        let error = db
            .command_recipe_plan(&discovery.source_root, "generated.copy")
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("trail-fixture-tool-that-does-not-exist"));
    }

    #[test]
    fn local_include_and_versioned_profile_expand_into_a_canonical_plan() {
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("config")).unwrap();
        fs::create_dir_all(workspace.path().join("apps/api")).unwrap();
        fs::write(
            workspace.path().join("apps/api/input.txt"),
            "profile input\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("trail.environment.toml"),
            r#"schema = "trail.environment/v1"
include = ["config/copy.toml"]

[environment]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "generated.profile-copy"
root = "apps/api"
extends = ["profile.copy"]
"#,
        )
        .unwrap();
        fs::write(
            workspace.path().join("config/copy.toml"),
            r#"schema = "trail.environment/v1"

[profile.copy]
version = "1.2.0"
adapter = "trail/command@1"
kind = "generated"
inputs = [{ path = "{root}/input.txt", role = "identity", format = "bytes" }]
outputs = [{ source = "generated", target = "{root}/generated", policy = "immutable_seed_private", portability = "host" }]

[profile.copy.build]
command = ["cp", "input.txt", "generated/copied.txt"]
cwd = "{root}"
network = "deny"
scripts = "deny"
"#,
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let root = db.resolve_branch_ref("main").unwrap().root_id;
        let plan = db
            .command_recipe_plan(&root, "generated.profile-copy")
            .unwrap();
        assert_eq!(
            plan.command.as_ref().unwrap().working_directory,
            "project/apps/api"
        );
        assert_eq!(plan.outputs[0].mount_path, "apps/api/generated");
        assert_eq!(plan.inputs[0].source_path, "apps/api/input.txt");
        assert_eq!(
            plan.layer_key
                .inputs
                .get("profile:copy")
                .map(String::as_str),
            Some("1.2.0")
        );
        assert!(plan
            .layer_key
            .inputs
            .contains_key("specification_source:trail.environment.toml"));
        assert!(plan
            .layer_key
            .inputs
            .contains_key("specification_source:config/copy.toml"));
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }

    #[test]
    fn maven_gradle_like_and_unknown_custom_shapes_use_repository_v2_components() {
        let specification = r#"schema = "trail.environment/v2"

[environment]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "jvm.dependencies"
adapter = "trail/command@1"
kind = "generated"
inputs = [{ path = "input.txt", role = "identity", format = "bytes" }]
outputs = [{ name = "dependencies", source = "dependencies", target = ".trail-generated/jvm-dependencies", policy = "immutable_seed_private", reuse = "exact", scope = "workspace", publish = "on_sync", portability = "host" }]
[component.build]
command = ["cp", "input.txt", "dependencies/checksums.lock"]
cwd = "."
network = "deny"
scripts = "deny"

[[component.validation]]
name = "dependency-checksum-graph"
kind = "path_contract"
path = "dependencies"
required = true
parameters = { maximum_entries = "10000" }

[[component]]
id = "jvm.private-build-state"
adapter = "trail/command@1"
kind = "generated"
depends_on = ["jvm.dependencies"]
inputs = [{ path = "input.txt", role = "identity", format = "bytes" }]
outputs = [{ name = "build-state", source = "build-state", target = ".trail-generated/jvm-build", policy = "writable_private", reuse = "none", scope = "lane", publish = "manual", portability = "host" }]
[component.build]
command = ["cp", "input.txt", "build-state/task-state.bin"]
cwd = "."
network = "deny"
scripts = "deny"

[[component]]
id = "custom.codegen"
adapter = "trail/command@1"
kind = "generated"
inputs = [{ path = "input.txt", role = "identity", format = "bytes" }]
outputs = [{ name = "generated-api", source = "generated-api", target = ".trail-generated/custom-api", policy = "immutable_seed_private", reuse = "exact", scope = "workspace", publish = "on_sync", portability = "host" }]
[component.build]
command = ["cp", "input.txt", "generated-api/client.txt"]
cwd = "."
network = "deny"
scripts = "deny"

[[component.validation]]
name = "generated-api-contract"
kind = "path_contract"
path = "generated-api"
required = true

[[component.source_export]]
from_output = "generated-api"
source = "client.txt"
target = "src/generated/client.txt"
mode = "explicit"
collision = "fail"
validation = "generated-api-contract"
"#;
        let (_workspace, db) = open_recipe_graph(specification);
        let source_root = db.resolve_branch_ref("main").unwrap().root_id;

        let dependencies = db
            .compile_repository_artifact_pipeline_v2(&source_root, "jvm.dependencies")
            .unwrap();
        let build_state = db
            .compile_repository_artifact_pipeline_v2(&source_root, "jvm.private-build-state")
            .unwrap();
        let custom = db
            .compile_repository_artifact_pipeline_v2(&source_root, "custom.codegen")
            .unwrap();

        for compiled in [&dependencies, &build_state, &custom] {
            assert_eq!(
                compiled.graph_plan.adapter_identity,
                RECIPE_ADAPTER_IDENTITY
            );
            assert_eq!(compiled.desired_material.adapter_protocol, RECIPE_SCHEMA_V2);
            assert_eq!(compiled.desired_material.trust_scope, "repository");
            assert_eq!(compiled.desired_material.network_policy, "deny");
        }
        assert_eq!(
            dependencies.outputs[0].policy,
            EnvironmentOutputPolicy::ImmutableSeedPrivate
        );
        assert_eq!(
            dependencies.validations[0].name,
            "dependency-checksum-graph"
        );
        assert_eq!(
            build_state.outputs[0].policy,
            EnvironmentOutputPolicy::WritablePrivate
        );
        assert_eq!(build_state.outputs[0].reuse, EnvironmentReuseMode::None);
        assert_eq!(
            build_state.graph_plan.dependencies,
            [WorkspaceEnvironmentDependency::build_requires(
                "jvm.dependencies"
            )]
        );
        assert_eq!(custom.source_exports.len(), 1);
        assert_eq!(
            custom.source_exports[0].destination,
            "src/generated/client.txt"
        );
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }

    #[test]
    fn component_dependencies_finalize_in_topological_order_and_fail_closed() {
        let chain = r#"schema = "trail.environment/v1"

[[component]]
id = "c"
adapter = "trail/command@1"
kind = "generated"
depends_on = ["b"]
inputs = [{ path = "input.txt" }]
outputs = [{ source = "out-c", target = ".trail-generated/c" }]
[component.build]
command = ["cp", "input.txt", "out-c/value.txt"]

[[component]]
id = "a"
adapter = "trail/command@1"
kind = "generated"
inputs = [{ path = "input.txt" }]
outputs = [{ source = "out-a", target = ".trail-generated/a" }]
[component.build]
command = ["cp", "input.txt", "out-a/value.txt"]

[[component]]
id = "b"
adapter = "trail/command@1"
kind = "generated"
depends_on = ["a"]
inputs = [{ path = "input.txt" }]
outputs = [{ source = "out-b", target = ".trail-generated/b" }]
[component.build]
command = ["cp", "input.txt", "out-b/value.txt"]
"#;
        let (_workspace, db) = open_recipe_graph(chain);
        let discovery = db.discover_workspace_environment("graph", None).unwrap();
        let finalized = db
            .plan_discovered_environment_graph(&discovery.source_root, &discovery.components)
            .unwrap();
        assert_eq!(
            finalized
                .iter()
                .map(|(plan, _)| plan.component_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(
            finalized[1].0.layer_key.inputs["dependency:a"],
            finalized[0].1
        );
        assert_eq!(
            finalized[2].0.layer_key.inputs["dependency:b"],
            finalized[1].1
        );
        let graph = db.workspace_environment_graph("graph", None).unwrap();
        assert_eq!(
            graph
                .nodes
                .iter()
                .map(|node| node.component_id.as_str())
                .collect::<Vec<_>>(),
            ["a", "b", "c"]
        );
        assert_eq!(graph.edges.len(), 2);
        assert_eq!(graph.edges[0].source_component_id, "a");
        assert_eq!(graph.edges[0].target_component_id, "b");
        assert_eq!(graph.edges[0].edge_type, "build_requires");
        assert_eq!(
            graph.edges[0].source_component_key,
            graph.nodes[0].component_key
        );
        assert_eq!(graph.edges[1].source_component_id, "b");
        assert_eq!(graph.edges[1].target_component_id, "c");
        assert!(db.list_workspace_layers().unwrap().is_empty());
        let report = db
            .plan_workspace_environment_component("graph", RECIPE_ADAPTER_IDENTITY, None, Some("c"))
            .unwrap();
        assert_eq!(report.dependencies, ["b"]);
        assert_eq!(report.component_key, finalized[2].1);
        let error = db
            .sync_workspace_environment_component("graph", RECIPE_ADAPTER_IDENTITY, None, Some("c"))
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("requires `b`, which is not attached"));
        assert!(error.to_string().contains("env sync all graph"));

        let missing = chain.replace("depends_on = [\"b\"]", "depends_on = [\"missing\"]");
        let (_workspace, db) = open_recipe_graph(&missing);
        let discovery = db.discover_workspace_environment("graph", None).unwrap();
        let error = db
            .plan_discovered_environment_graph(&discovery.source_root, &discovery.components)
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("component `c` requires missing component `missing`"));

        let cycle = chain.replace(
            "id = \"a\"\nadapter = \"trail/command@1\"\nkind = \"generated\"",
            "id = \"a\"\nadapter = \"trail/command@1\"\nkind = \"generated\"\ndepends_on = [\"c\"]",
        );
        let (_workspace, db) = open_recipe_graph(&cycle);
        let discovery = db.discover_workspace_environment("graph", None).unwrap();
        let error = db
            .plan_discovered_environment_graph(&discovery.source_root, &discovery.components)
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("dependency cycle: a -> c -> b -> a"));
    }

    #[test]
    fn recipe_typed_edges_are_reported_and_only_identity_edges_change_keys() {
        let specification = r#"schema = "trail.environment/v1"

[[component]]
id = "source"
adapter = "trail/command@1"
kind = "generated"
inputs = [{ path = "input.txt" }]
outputs = [{ source = "out-source", target = ".trail-generated/source" }]
[component.build]
command = ["cp", "input.txt", "out-source/value.txt"]

[[component]]
id = "runtime"
adapter = "trail/command@1"
kind = "generated"
inputs = [{ path = "input.txt" }]
outputs = [{ source = "out-runtime", target = ".trail-generated/runtime" }]
[[component.edge]]
component = "source"
type = "runtime_requires"
[component.build]
command = ["cp", "input.txt", "out-runtime/value.txt"]

[[component]]
id = "configuration"
adapter = "trail/command@1"
kind = "generated"
inputs = [{ path = "input.txt" }]
outputs = [{ source = "out-configuration", target = ".trail-generated/configuration" }]
[[component.edge]]
component = "source"
type = "invalidates_with"
[component.build]
command = ["cp", "input.txt", "out-configuration/value.txt"]
"#;
        let (_workspace, db) = open_recipe_graph(specification);
        let discovery = db.discover_workspace_environment("graph", None).unwrap();
        let finalized = db
            .plan_discovered_environment_graph(&discovery.source_root, &discovery.components)
            .unwrap();
        let by_id = finalized
            .iter()
            .map(|(plan, key)| (plan.component_id.as_str(), (plan, key)))
            .collect::<BTreeMap<_, _>>();
        assert!(!by_id["runtime"]
            .0
            .layer_key
            .inputs
            .keys()
            .any(|key| key.starts_with("dependency:")));
        assert_eq!(
            by_id["configuration"].0.layer_key.inputs["dependency:invalidates_with:source"],
            *by_id["source"].1
        );
        let graph = db.workspace_environment_graph("graph", None).unwrap();
        assert_eq!(
            graph
                .edges
                .iter()
                .map(|edge| (edge.target_component_id.as_str(), edge.edge_type.as_str()))
                .collect::<Vec<_>>(),
            [
                ("configuration", "invalidates_with"),
                ("runtime", "runtime_requires")
            ]
        );
    }

    #[test]
    fn thousand_component_graph_parses_recipes_twice_not_once_per_component() {
        let count = 1_000usize;
        let program = if cfg!(windows) { "where" } else { "cp" };
        let mut specification = String::from("schema = \"trail.environment/v1\"\n");
        for index in (0..count).rev() {
            let component_id = format!("component-{index:04}");
            let dependency = if index > 0 {
                format!("depends_on = [\"component-{:04}\"]\n", index - 1)
            } else {
                Default::default()
            };
            specification.push_str(&format!(
                r#"
[[component]]
id = "{component_id}"
adapter = "trail/command@1"
kind = "generated"
{dependency}inputs = [{{ path = "input.txt" }}]
outputs = [{{ source = "out-{index:04}", target = ".trail-generated/{component_id}" }}]
[component.build]
command = ["{program}", "input.txt", "out-{index:04}/value.txt"]
"#
            ));
        }
        let (_workspace, db) = open_recipe_graph(&specification);
        COMMAND_RECIPE_LOAD_COUNT.with(|loads| loads.set(0));
        let graph = db.workspace_environment_graph("graph", None).unwrap();
        assert_eq!(graph.nodes.len(), count);
        assert_eq!(graph.edges.len(), count - 1);
        assert_eq!(graph.nodes[0].component_id, "component-0000");
        assert_eq!(graph.nodes[count - 1].component_id, "component-0999");
        COMMAND_RECIPE_LOAD_COUNT.with(|loads| assert_eq!(loads.get(), 2));
        let page = db
            .workspace_environment_graph_page("graph", None, 400, 250)
            .unwrap();
        assert_eq!(page.total_nodes, count as u64);
        assert_eq!(page.total_edges, (count - 1) as u64);
        assert_eq!(page.offset, 400);
        assert_eq!(page.next_offset, Some(650));
        assert_eq!(page.nodes.len(), 250);
        assert_eq!(page.edges.len(), 250);
        assert_eq!(page.nodes[0].component_id, "component-0400");
        COMMAND_RECIPE_LOAD_COUNT.with(|loads| assert_eq!(loads.get(), 4));
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }

    #[test]
    fn recipe_include_and_profile_cycles_fail_with_the_full_chain() {
        let include_workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(include_workspace.path().join("config")).unwrap();
        fs::write(
            include_workspace.path().join("trail.environment.toml"),
            "schema = \"trail.environment/v1\"\ninclude = [\"config/a.toml\"]\n",
        )
        .unwrap();
        fs::write(
            include_workspace.path().join("config/a.toml"),
            "schema = \"trail.environment/v1\"\ninclude = [\"b.toml\"]\n",
        )
        .unwrap();
        fs::write(
            include_workspace.path().join("config/b.toml"),
            "schema = \"trail.environment/v1\"\ninclude = [\"a.toml\"]\n",
        )
        .unwrap();
        Trail::init(
            include_workspace.path(),
            "main",
            InitImportMode::WorkingTree,
            false,
        )
        .unwrap();
        let include_db = Trail::open(include_workspace.path()).unwrap();
        let root = include_db.resolve_branch_ref("main").unwrap().root_id;
        let error = include_db.load_command_recipes(&root).unwrap_err();
        assert!(error
            .to_string()
            .contains("config/a.toml -> config/b.toml -> config/a.toml"));

        let profile_workspace = tempfile::tempdir().unwrap();
        fs::write(
            profile_workspace.path().join("trail.environment.toml"),
            r#"schema = "trail.environment/v1"

[profile.a]
version = "1"
extends = ["profile.b"]

[profile.b]
version = "1"
extends = ["profile.a"]

[[component]]
id = "generated.cycle"
extends = ["profile.a"]
"#,
        )
        .unwrap();
        Trail::init(
            profile_workspace.path(),
            "main",
            InitImportMode::WorkingTree,
            false,
        )
        .unwrap();
        let profile_db = Trail::open(profile_workspace.path()).unwrap();
        let root = profile_db.resolve_branch_ref("main").unwrap().root_id;
        let error = profile_db.load_command_recipes(&root).unwrap_err();
        assert!(error.to_string().contains("a -> b -> a"));
    }

    #[test]
    fn recipe_includes_reject_remote_globbed_and_traversing_paths() {
        for include in ["https://example.invalid/x.toml", "*.toml", "../x.toml"] {
            let workspace = tempfile::tempdir().unwrap();
            fs::write(
                workspace.path().join("trail.environment.toml"),
                format!("schema = \"trail.environment/v1\"\ninclude = [{include:?}]\n"),
            )
            .unwrap();
            Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let db = Trail::open(workspace.path()).unwrap();
            let root = db.resolve_branch_ref("main").unwrap().root_id;
            let error = db.load_command_recipes(&root).unwrap_err();
            assert!(error
                .to_string()
                .contains("invalid local environment specification include"));
        }
    }

    #[test]
    fn command_recipe_rejects_shells_before_execution() {
        let (_workspace, db) = open_recipe_lane(&["sh", "-c", "true"]);
        let discovery = db.discover_workspace_environment("recipe-a", None).unwrap();
        let error = db
            .command_recipe_plan(&discovery.source_root, "generated.copy")
            .unwrap_err();
        assert!(error.to_string().contains("non-shell executable"));
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }

    #[test]
    fn command_recipe_component_selector_disambiguates_shared_roots() {
        let workspace = tempfile::tempdir().unwrap();
        write_recipe_workspace(
            workspace.path(),
            &["cp", "input.txt", "generated/copied.txt"],
        );
        let mut specification =
            fs::read_to_string(workspace.path().join("trail.environment.toml")).unwrap();
        specification.push_str(
            r#"
[[component]]
id = "generated.second"
adapter = "trail/command@1"
root = "."
kind = "generated"
inputs = [{ path = "input.txt", role = "identity", format = "bytes" }]

[component.build]
command = ["cp", "input.txt", "generated-second/copied.txt"]
cwd = "."
network = "deny"
scripts = "deny"

[[component.output]]
source = "generated-second"
target = ".trail-generated/second"
policy = "immutable_seed_private"
portability = "host"
"#,
        );
        fs::write(
            workspace.path().join("trail.environment.toml"),
            specification,
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "recipes",
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
        let error = db
            .plan_workspace_environment("recipes", RECIPE_ADAPTER_IDENTITY, None)
            .unwrap_err();
        assert!(error.to_string().contains("2 `trail/command@1` components"));
        let selected = db
            .plan_workspace_environment_component(
                "recipes",
                RECIPE_ADAPTER_IDENTITY,
                None,
                Some("generated.second"),
            )
            .unwrap();
        assert_eq!(selected.component_id, "generated.second");
        assert_eq!(selected.mount_path, ".trail-generated/second");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn restricted_command_recipe_builds_once_and_reuses_a_verified_layer() {
        let (_workspace, db) = open_recipe_lane(&["cp", "input.txt", "generated/copied.txt"]);
        let first_batch = db
            .sync_all_workspace_environments("recipe-a", None)
            .unwrap();
        assert_eq!(first_batch.generation.components.len(), 1);
        assert_eq!(
            first_batch.generation.components[0].component_id,
            "generated.copy"
        );
        let first = &first_batch.layers[0];
        let second = db
            .sync_workspace_environment("recipe-b", "command", None)
            .unwrap();
        assert_eq!(first.layer_id, second.layer_id);
        assert_eq!(first.adapter, "command");
        assert_eq!(
            fs::read(Path::new(&first.storage_path).join("copied.txt")).unwrap(),
            b"declared input\n"
        );
        assert_eq!(db.list_workspace_layers().unwrap().len(), 1);
        for lane in ["recipe-a", "recipe-b"] {
            let status = db.environment_component_status(lane).unwrap();
            assert_eq!(status[0].status, "ready");
            assert_eq!(status[0].component.kind, "generated");
            assert_eq!(status[0].adapter.name, "command");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn writable_private_recipe_has_no_fake_layer_and_preserves_compatible_lane_state() {
        let workspace = tempfile::tempdir().unwrap();
        write_recipe_workspace_with_policy(
            workspace.path(),
            &["cp", "input.txt", "generated/copied.txt"],
            "writable_private",
        );
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        for lane in ["private-a", "private-b"] {
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                lane,
                Some("main"),
                LaneWorkdirMode::NfsCow,
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
        }

        let plan = db
            .plan_workspace_environment("private-a", RECIPE_ADAPTER_IDENTITY, None)
            .unwrap();
        assert_eq!(
            plan.outputs[0].policy,
            EnvironmentOutputPolicy::WritablePrivate
        );
        let first = db
            .sync_workspace_environment_component("private-a", RECIPE_ADAPTER_IDENTITY, None, None)
            .unwrap();
        assert!(first.layers.is_empty());
        let output = &first.generation.components[0].outputs[0];
        assert_eq!(output.policy, EnvironmentOutputPolicy::WritablePrivate);
        assert!(output.layer_id.is_none());
        assert!(output.storage_identity.starts_with("private_"));
        assert!(db.list_workspace_layers().unwrap().is_empty());
        assert_eq!(
            db.workspace_layer_key_by_cache_key(&plan.component_key)
                .unwrap()
                .unwrap()
                .strategy,
            "restricted-command-recipe-v1"
        );

        let mounted = db.mount_nfs_cow_workdir_for_lane("private-a").unwrap();
        let workdir = PathBuf::from(db.lane_workdir("private-a").unwrap().workdir.unwrap());
        let copied = workdir.join(".trail-generated/copy/copied.txt");
        assert_eq!(fs::read(&copied).unwrap(), b"declared input\n");
        fs::write(&copied, "lane-private mutation\n").unwrap();
        drop(mounted);

        let second = db
            .sync_workspace_environment_component("private-a", RECIPE_ADAPTER_IDENTITY, None, None)
            .unwrap();
        assert!(second.layers.is_empty());
        assert_eq!(
            second.generation.predecessor_generation_id.as_deref(),
            Some(first.generation.generation_id.as_str())
        );
        let mounted = db.mount_nfs_cow_workdir_for_lane("private-a").unwrap();
        assert_eq!(fs::read(&copied).unwrap(), b"lane-private mutation\n");
        fs::write(workdir.join("input.txt"), "changed input\n").unwrap();
        drop(mounted);
        db.checkpoint_lane_workspace("private-a", Some("change private input".to_string()))
            .unwrap();
        let readiness = db.lane_readiness("private-a").unwrap();
        assert!(readiness
            .blockers
            .iter()
            .any(|blocker| blocker.code == "dependency_environment_stale"));
        let explanation = db
            .explain_workspace_environment_staleness("private-a", "generated.copy")
            .unwrap();
        assert!(explanation.provenance_complete);
        assert!(
            explanation.changes.iter().any(|change| {
                change.dimension == "input"
                    && change.name == "input.txt"
                    && change.change == "modified"
            }),
            "{:?}",
            explanation.changes
        );
        let rebuilt = db
            .sync_workspace_environment_component("private-a", RECIPE_ADAPTER_IDENTITY, None, None)
            .unwrap();
        assert!(rebuilt.layers.is_empty());
        let mounted = db.mount_nfs_cow_workdir_for_lane("private-a").unwrap();
        assert_eq!(fs::read(&copied).unwrap(), b"changed input\n");
        fs::remove_dir_all(workdir.join(".trail-generated/copy")).unwrap();
        drop(mounted);
        let restored = db
            .sync_workspace_environment_component("private-a", RECIPE_ADAPTER_IDENTITY, None, None)
            .unwrap();
        assert!(restored.layers.is_empty());
        let mounted = db.mount_nfs_cow_workdir_for_lane("private-a").unwrap();
        assert_eq!(fs::read(&copied).unwrap(), b"changed input\n");
        drop(mounted);

        let other = db
            .sync_workspace_environment_component("private-b", RECIPE_ADAPTER_IDENTITY, None, None)
            .unwrap();
        assert!(other.layers.is_empty());
        let mounted = db.mount_nfs_cow_workdir_for_lane("private-b").unwrap();
        let other_workdir = PathBuf::from(db.lane_workdir("private-b").unwrap().workdir.unwrap());
        assert_eq!(
            fs::read(other_workdir.join(".trail-generated/copy/copied.txt")).unwrap(),
            b"declared input\n"
        );
        drop(mounted);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn manual_private_output_promotion_is_journaled_and_preserves_private_bytes() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("input.txt"), "promotion input\n").unwrap();
        fs::write(
            workspace.path().join("trail.environment.toml"),
            r#"schema = "trail.environment/v1"

[environment]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "generated.promotable"
adapter = "trail/command@1"
kind = "generated"
inputs = [{ path = "input.txt" }]
outputs = [{ name = "result", source = "out", target = ".trail-generated/promotable", policy = "writable_private", reuse = "none", scope = "lane", publish = "manual" }]

[component.build]
command = ["cp", "input.txt", "out/value.txt"]
network = "deny"
scripts = "deny"
"#,
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "promotion",
            Some("main"),
            LaneWorkdirMode::NfsCow,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let synchronized = db
            .sync_all_workspace_environments("promotion", None)
            .unwrap();
        let predecessor = synchronized.generation.generation_id;
        let view = db.lane_workspace_view("promotion").unwrap().unwrap();
        let private_file =
            Path::new(&view.generated_upper).join(".trail-generated/promotable/value.txt");
        fs::write(&private_file, "lane-private promoted bytes\n").unwrap();

        let promoted = db
            .promote_workspace_environment_output("promotion", "generated.promotable", "result")
            .unwrap();
        assert_eq!(promoted.phase, "activated");
        assert_eq!(promoted.predecessor_generation_id, predecessor);
        assert_ne!(promoted.successor_generation_id, predecessor);
        assert_eq!(
            fs::read_to_string(&private_file).unwrap(),
            "lane-private promoted bytes\n"
        );
        assert_eq!(
            fs::read_to_string(Path::new(&promoted.layer.storage_path).join("value.txt")).unwrap(),
            "lane-private promoted bytes\n"
        );
        let generation = db
            .active_environment_generation("promotion")
            .unwrap()
            .unwrap();
        let output = &generation.components[0].outputs[0];
        assert_eq!(
            output.publication_id.as_deref(),
            Some(promoted.publication_id.as_str())
        );
        assert_eq!(
            output.layer_id.as_deref(),
            Some(promoted.layer.layer_id.as_str())
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn sync_all_atomically_composes_shared_and_private_components() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("input.txt"), "composed\n").unwrap();
        fs::write(workspace.path().join("private-input.txt"), "private base\n").unwrap();
        fs::write(
            workspace.path().join("trail.environment.toml"),
            r#"schema = "trail.environment/v1"

[environment]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "generated.shared"
adapter = "trail/command@1"
root = "."
kind = "generated"
inputs = [{ path = "input.txt", role = "identity", format = "bytes" }]
outputs = [{ name = "shared", source = "shared", target = ".trail-generated/shared", policy = "immutable_seed_private", portability = "host" }]
[component.build]
command = ["cp", "input.txt", "shared/value.txt"]
cwd = "."
network = "deny"
scripts = "deny"

[[component]]
id = "generated.private"
adapter = "trail/command@1"
root = "."
kind = "generated"
depends_on = ["generated.shared"]
inputs = [{ path = "private-input.txt", role = "identity", format = "bytes" }]
outputs = [{ name = "private", source = "private", target = ".trail-generated/private", policy = "writable_private", portability = "host" }]
[component.build]
command = ["cp", "private-input.txt", "private/value.txt"]
cwd = "."
network = "deny"
scripts = "deny"
"#,
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "composed",
            Some("main"),
            LaneWorkdirMode::NfsCow,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();

        let first = db
            .sync_all_workspace_environments("composed", None)
            .unwrap();
        assert_eq!(first.layers.len(), 1);
        assert_eq!(first.generation.components.len(), 2);
        let private_component = first
            .generation
            .components
            .iter()
            .find(|component| component.component_id == "generated.private")
            .unwrap();
        assert_eq!(private_component.dependencies.len(), 1);
        assert_eq!(
            private_component.dependencies[0].component_id,
            "generated.shared"
        );
        assert_eq!(
            private_component.dependencies[0].edge_type,
            "build_requires"
        );
        assert_eq!(
            private_component.dependencies[0].component_key,
            first
                .generation
                .components
                .iter()
                .find(|component| component.component_id == "generated.shared")
                .unwrap()
                .component_key
        );
        let policies = first
            .generation
            .components
            .iter()
            .flat_map(|component| &component.outputs)
            .map(|output| output.policy.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            policies,
            BTreeSet::from(["immutable_seed_private", "writable_private"])
        );
        let mounted = db.mount_nfs_cow_workdir_for_lane("composed").unwrap();
        let workdir = PathBuf::from(db.lane_workdir("composed").unwrap().workdir.unwrap());
        assert_eq!(
            fs::read(workdir.join(".trail-generated/shared/value.txt")).unwrap(),
            b"composed\n"
        );
        let private = workdir.join(".trail-generated/private/value.txt");
        assert_eq!(fs::read(&private).unwrap(), b"private base\n");
        fs::write(&private, "preserved private\n").unwrap();
        drop(mounted);

        let second = db
            .sync_all_workspace_environments("composed", None)
            .unwrap();
        assert_eq!(second.layers.len(), 1);
        assert_eq!(second.layers[0].layer_id, first.layers[0].layer_id);
        let mounted = db.mount_nfs_cow_workdir_for_lane("composed").unwrap();
        assert_eq!(fs::read(&private).unwrap(), b"preserved private\n");
        drop(mounted);

        let mounted = db.mount_nfs_cow_workdir_for_lane("composed").unwrap();
        fs::write(workdir.join("input.txt"), "changed upstream\n").unwrap();
        drop(mounted);
        db.checkpoint_lane_workspace("composed", Some("change upstream".to_string()))
            .unwrap();
        let readiness = db.lane_readiness("composed").unwrap();
        assert!(readiness
            .blockers
            .iter()
            .any(|blocker| blocker.code == "dependency_environment_stale"));
        let explanation = db
            .explain_workspace_environment_staleness("composed", "generated.private")
            .unwrap();
        assert!(explanation.changes.iter().any(|change| {
            change.dimension == "input"
                && change.name == "dependency:generated.shared"
                && change.change == "modified"
        }));
        let old_private_dependency_key = second
            .generation
            .components
            .iter()
            .find(|component| component.component_id == "generated.private")
            .unwrap()
            .dependencies[0]
            .component_key
            .clone();
        let upstream_only = db
            .sync_workspace_environment_component(
                "composed",
                RECIPE_ADAPTER_IDENTITY,
                None,
                Some("generated.shared"),
            )
            .unwrap();
        let private_after_upstream = upstream_only
            .generation
            .components
            .iter()
            .find(|component| component.component_id == "generated.private")
            .unwrap();
        assert_eq!(
            private_after_upstream.dependencies[0].component_key,
            old_private_dependency_key
        );
        assert_eq!(
            db.environment_component_status("composed")
                .unwrap()
                .into_iter()
                .find(|state| state.component.component_id == "generated.private")
                .unwrap()
                .status,
            "stale"
        );
        let rebuilt = db
            .sync_all_workspace_environments("composed", None)
            .unwrap();
        assert!(rebuilt
            .generation
            .components
            .iter()
            .any(|component| component.component_id == "generated.private"));
        let mounted = db.mount_nfs_cow_workdir_for_lane("composed").unwrap();
        assert_eq!(fs::read(&private).unwrap(), b"private base\n");
        drop(mounted);

        let mounted = db.mount_nfs_cow_workdir_for_lane("composed").unwrap();
        let specification_path = workdir.join("trail.environment.toml");
        let specification = fs::read_to_string(&specification_path).unwrap();
        let retained = specification
            .split_once("\n[[component]]\nid = \"generated.private\"")
            .unwrap()
            .0;
        fs::write(&specification_path, format!("{retained}\n")).unwrap();
        drop(mounted);
        db.checkpoint_lane_workspace("composed", Some("remove private component".to_string()))
            .unwrap();
        let retired = db
            .sync_all_workspace_environments("composed", None)
            .unwrap();
        assert_eq!(retired.generation.components.len(), 1);
        assert_eq!(
            retired.generation.components[0].component_id,
            "generated.shared"
        );
        assert!(db
            .environment_component_status("composed")
            .unwrap()
            .into_iter()
            .all(|state| state.component.component_id != "generated.private"));
        let mounted = db.mount_nfs_cow_workdir_for_lane("composed").unwrap();
        assert!(!workdir.join(".trail-generated/private").exists());
        drop(mounted);

        let mounted = db.mount_nfs_cow_workdir_for_lane("composed").unwrap();
        fs::remove_file(workdir.join("trail.environment.toml")).unwrap();
        drop(mounted);
        db.checkpoint_lane_workspace("composed", Some("remove environment".to_string()))
            .unwrap();
        let cleared = db
            .sync_all_workspace_environments("composed", None)
            .unwrap();
        assert!(cleared.generation.components.is_empty());
        assert!(cleared.layers.is_empty());
        assert!(db
            .environment_component_status("composed")
            .unwrap()
            .is_empty());
        let mounted = db.mount_nfs_cow_workdir_for_lane("composed").unwrap();
        assert!(!workdir.join(".trail-generated/shared").exists());
        drop(mounted);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn restricted_command_recipe_publishes_and_activates_multiple_outputs_atomically() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("input.txt"), "identity\n").unwrap();
        fs::write(
            workspace.path().join("trail.environment.toml"),
            r#"schema = "trail.environment/v1"

[environment]
default_network = "deny"
default_scripts = "deny"

[[component]]
id = "generated.multi"
adapter = "trail/command@1"
kind = "generated"
inputs = [{ path = "input.txt", role = "identity", format = "bytes" }]

[component.build]
command = ["touch", "generated-a/a.txt", "generated-b/b.txt"]
cwd = "."
network = "deny"
scripts = "deny"

[[component.output]]
name = "alpha"
source = "generated-a"
target = ".trail-generated/alpha"
policy = "immutable_seed_private"
portability = "host"

[[component.output]]
name = "beta"
source = "generated-b"
target = ".trail-generated/beta"
policy = "immutable_seed_private"
portability = "host"
"#,
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        for lane in ["multi-a", "multi-b"] {
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                lane,
                Some("main"),
                LaneWorkdirMode::NfsCow,
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
        }

        let planned = db
            .plan_workspace_environment_component(
                "multi-a",
                RECIPE_ADAPTER_IDENTITY,
                None,
                Some("generated.multi"),
            )
            .unwrap();
        assert_eq!(planned.outputs.len(), 2);
        assert_eq!(planned.capabilities.filesystem_write.len(), 2);

        let first = db.sync_all_workspace_environments("multi-a", None).unwrap();
        assert_eq!(first.layers.len(), 1);
        assert_eq!(first.generation.components.len(), 1);
        let component = &first.generation.components[0];
        assert_eq!(component.outputs.len(), 2);
        assert_eq!(component.outputs[0].name, "alpha");
        assert_eq!(component.outputs[1].name, "beta");
        assert_eq!(component.outputs[0].layer_id, component.outputs[1].layer_id);
        let layer_root = Path::new(&first.layers[0].storage_path);
        assert!(layer_root.join("outputs/0000/a.txt").is_file());
        assert!(layer_root.join("outputs/0001/b.txt").is_file());

        let output_rows = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM environment_component_output_bindings WHERE component_id = 'generated.multi'",
                [],
                |row| row.get::<_, usize>(0),
            )
            .unwrap();
        assert_eq!(output_rows, 2);
        let second = db
            .sync_workspace_environment_component(
                "multi-b",
                RECIPE_ADAPTER_IDENTITY,
                None,
                Some("generated.multi"),
            )
            .unwrap();
        assert_eq!(first.layers[0].layer_id, second.layers[0].layer_id);
        let second_generation = db
            .active_environment_generation("multi-b")
            .unwrap()
            .unwrap();
        assert_eq!(second_generation.components[0].outputs.len(), 2);

        let mount_a = db.mount_nfs_cow_workdir_for_lane("multi-a").unwrap();
        let mount_b = db.mount_nfs_cow_workdir_for_lane("multi-b").unwrap();
        let workdir_a = PathBuf::from(db.lane_workdir("multi-a").unwrap().workdir.unwrap());
        let workdir_b = PathBuf::from(db.lane_workdir("multi-b").unwrap().workdir.unwrap());
        let alpha_a = workdir_a.join(".trail-generated/alpha/a.txt");
        let beta_a = workdir_a.join(".trail-generated/beta/b.txt");
        let alpha_b = workdir_b.join(".trail-generated/alpha/a.txt");
        let beta_b = workdir_b.join(".trail-generated/beta/b.txt");
        assert_eq!(fs::read(&alpha_a).unwrap(), b"");
        assert_eq!(fs::read(&beta_a).unwrap(), b"");
        fs::write(&alpha_a, b"lane-a").unwrap();
        fs::write(&beta_a, b"private-beta").unwrap();
        assert_eq!(fs::read(&alpha_a).unwrap(), b"lane-a");
        assert_eq!(fs::read(&beta_a).unwrap(), b"private-beta");
        assert_eq!(fs::read(&alpha_b).unwrap(), b"");
        assert_eq!(fs::read(&beta_b).unwrap(), b"");
        drop(mount_a);
        drop(mount_b);
        db.replace_declared_workspace_layers(
            "multi-a",
            &[EnvironmentLayerActivation {
                layer_id: Some(first.layers[0].layer_id.clone()),
                outputs: vec![EnvironmentLayerOutputActivation {
                    name: "alpha".to_string(),
                    mount_path: ".trail-generated/alpha".to_string(),
                    policy: EnvironmentOutputPolicy::ImmutableSeedPrivate,
                    reuse: EnvironmentReuseMode::Exact,
                    scope: EnvironmentSharingScope::Workspace,
                    publish: EnvironmentPublicationTrigger::OnSync,
                    gate: None,
                    binding_identity: first.layers[0].layer_id.clone(),
                    manifest_object_id: None,
                    publication_id: None,
                    private_seed: None,
                    layer_subpath: "outputs/0000".to_string(),
                }],
                component_id: "generated.multi".to_string(),
                adapter_identity: RECIPE_ADAPTER_IDENTITY.to_string(),
                adapter_version: 1,
                implementation_version: env!("CARGO_PKG_VERSION").to_string(),
                distribution_digest: "builtin:command-recipe-plan-v1".to_string(),
                kind: "generated".to_string(),
                dependencies: Vec::new(),
                caches: Vec::new(),
                external_artifacts: Vec::new(),
                runtime_resources: Vec::new(),
                expected_key: first.layers[0].cache_key.clone(),
                canonical_key: db
                    .workspace_layer_key_by_cache_key(&first.layers[0].cache_key)
                    .unwrap()
                    .unwrap(),
            }],
        )
        .unwrap();
        let reduced = db
            .active_environment_generation("multi-a")
            .unwrap()
            .unwrap();
        assert_eq!(reduced.components[0].outputs.len(), 1);
        let view = db.lane_workspace_view("multi-a").unwrap().unwrap();
        let generated_upper = Path::new(&view.source_upper)
            .parent()
            .unwrap()
            .join("generated-upper/.trail-generated/beta");
        assert!(!generated_upper.exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn restricted_command_recipe_denies_undeclared_host_reads() {
        let (_workspace, db) = open_recipe_lane(&["cp", "/etc/passwd", "generated/copied.txt"]);
        let error = db
            .sync_workspace_environment("recipe-a", RECIPE_ADAPTER_IDENTITY, None)
            .unwrap_err();
        assert!(error.to_string().contains("failed with"));
        assert!(db.list_workspace_layers().unwrap().is_empty());
        let status = db.environment_component_status("recipe-a").unwrap();
        assert_eq!(status[0].status, "failed");
        assert_eq!(status[0].attached_key, None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn restricted_command_recipe_denies_writes_outside_declared_output() {
        let (_workspace, db) = open_recipe_lane(&["cp", "input.txt", "escape.txt"]);
        let error = db
            .sync_workspace_environment("recipe-a", RECIPE_ADAPTER_IDENTITY, None)
            .unwrap_err();
        assert!(error.to_string().contains("failed with"));
        assert!(db.list_workspace_layers().unwrap().is_empty());
        assert!(db
            .active_environment_generation("recipe-a")
            .unwrap()
            .is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn restricted_command_recipe_denies_network_connections() {
        use std::net::TcpListener;
        use std::thread;
        use std::time::{Duration, Instant};

        if !Path::new("/usr/bin/nc").is_file() {
            return;
        }
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let address = listener.local_addr().unwrap();
        let observer = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(3);
            while Instant::now() < deadline {
                match listener.accept() {
                    Ok((_stream, _)) => return true,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => return false,
                }
            }
            false
        });
        let port = address.port().to_string();
        let (_workspace, db) = open_recipe_lane(&["nc", "-z", "-w", "1", "127.0.0.1", &port]);
        let error = db
            .sync_workspace_environment("recipe-a", RECIPE_ADAPTER_IDENTITY, None)
            .unwrap_err();
        assert!(error.to_string().contains("failed with"));
        assert!(
            !observer.join().unwrap(),
            "sandboxed netcat reached a host socket"
        );
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn restricted_command_recipe_denies_child_process_execution() {
        let (_workspace, db) =
            open_recipe_lane(&["env", "cp", "input.txt", "generated/copied-by-child.txt"]);
        let error = db
            .sync_workspace_environment("recipe-a", RECIPE_ADAPTER_IDENTITY, None)
            .unwrap_err();
        assert!(error.to_string().contains("failed with"));
        assert!(db.list_workspace_layers().unwrap().is_empty());
        assert!(db
            .active_environment_generation("recipe-a")
            .unwrap()
            .is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn restricted_command_recipe_never_publishes_an_escaping_symlink() {
        let (_workspace, db) =
            open_recipe_lane(&["ln", "-s", "/etc/passwd", "generated/passwd-link"]);
        let error = db
            .sync_workspace_environment("recipe-a", RECIPE_ADAPTER_IDENTITY, None)
            .unwrap_err();
        assert!(error.to_string().contains("symlink"));
        assert!(db
            .list_workspace_layers()
            .unwrap()
            .iter()
            .all(|layer| layer.state != "available"));
        assert!(db
            .active_environment_generation("recipe-a")
            .unwrap()
            .is_none());
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn restricted_command_recipe_fails_closed_without_a_kernel_backend() {
        let (_workspace, db) = open_recipe_lane(&["cp", "input.txt", "generated/copied.txt"]);
        let error = db
            .sync_workspace_environment("recipe-a", RECIPE_ADAPTER_IDENTITY, None)
            .unwrap_err();
        assert!(error.to_string().contains("sandboxing is unavailable"));
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }
}
