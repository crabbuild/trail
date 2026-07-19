use globset::{GlobBuilder, GlobSetBuilder};
use serde::{Deserialize, Serialize};

use super::workspace_environment::{
    resolve_workspace_tool_executable, ResolvedWorkspaceTool, WorkspaceEnvironmentAdapterMetadata,
    WorkspaceEnvironmentCommand, WorkspaceEnvironmentDependency, WorkspaceEnvironmentEdgeType,
    WorkspaceEnvironmentInput, WorkspaceEnvironmentOutput, WorkspaceEnvironmentOutputPolicy,
    WorkspaceEnvironmentPlan, WorkspaceEnvironmentSandboxPolicy,
};
use super::*;

const RECIPE_SCHEMA: &str = "trail.environment/v1";
const RECIPE_ADAPTER_IDENTITY: &str = "trail/command@1";
const RECIPE_SPEC_PATHS: [&str; 2] = ["trail.environment.toml", ".trail/environment.toml"];
const MAX_RECIPE_SPEC_BYTES: u64 = 1024 * 1024;
const MAX_RECIPE_TOTAL_SPEC_BYTES: u64 = 4 * 1024 * 1024;
const MAX_RECIPE_INCLUDE_FILES: usize = 32;
const MAX_RECIPE_INCLUDE_DEPTH: usize = 8;
const MAX_RECIPE_INPUT_FILES: usize = 100_000;
const MAX_RECIPE_INPUT_BYTES: u64 = 2 * 1024 * 1024 * 1024;

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
    specification_digest: String,
    specification_sources: BTreeMap<String, String>,
    profile_versions: BTreeMap<String, String>,
    defaults: RecipeEnvironment,
    component: RecipeComponent,
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
}

impl Default for RecipeEnvironment {
    fn default() -> Self {
        Self {
            name: None,
            default_network: "deny".to_string(),
            default_scripts: "deny".to_string(),
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
}

#[derive(Clone, Debug)]
struct ResolvedRecipeProfile {
    fragment: RecipeFragment,
    versions: BTreeMap<String, String>,
}

#[derive(Debug, Default)]
struct RecipeDocuments {
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
    #[serde(default = "default_private_seed_policy")]
    policy: String,
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

fn default_recipe_kind() -> String {
    "generated".to_string()
}

fn default_identity_role() -> String {
    "identity".to_string()
}

fn default_bytes_format() -> String {
    "bytes".to_string()
}

fn default_private_seed_policy() -> String {
    "immutable_seed_private".to_string()
}

fn default_host_portability() -> String {
    "host".to_string()
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
            .map(|recipe| EnvironmentDiscoveredComponentReport {
                component_id: recipe.component.id,
                component_root: recipe.component.root,
                kind: recipe.component.kind,
                adapter_identity: RECIPE_ADAPTER_IDENTITY.to_string(),
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
        self.plan_command_recipe(source_root, recipe)
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
            1 => self.plan_command_recipe(source_root, matching.remove(0)),
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
            let canonical = serde_json::to_vec(&(RECIPE_SCHEMA, &component, &profile_versions))?;
            recipes.push(CommandRecipe {
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
        validate_recipe_specification_header(&specification, path)?;

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
            let policy = match output.policy.as_str() {
                "immutable_seed_private" => WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate,
                "writable_private" => WorkspaceEnvironmentOutputPolicy::WritablePrivate,
                _ => {
                    return Err(Error::InvalidInput(format!(
                        "command component `{}` output policy must be `immutable_seed_private` or `writable_private`",
                        component.id
                    )));
                }
            };
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
) -> Result<()> {
    if specification.schema != RECIPE_SCHEMA {
        return Err(Error::InvalidInput(format!(
            "unsupported environment schema `{}` in `{path}`; expected `{RECIPE_SCHEMA}`",
            specification.schema
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
    Ok(())
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
        assert!(error.to_string().contains("env sync-all graph"));

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
        assert_eq!(plan.outputs[0].policy, "writable_private");
        let first = db
            .sync_workspace_environment_component("private-a", RECIPE_ADAPTER_IDENTITY, None, None)
            .unwrap();
        assert!(first.layers.is_empty());
        let output = &first.generation.components[0].outputs[0];
        assert_eq!(output.policy, "writable_private");
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
                    policy: "immutable_seed_private".to_string(),
                    binding_identity: first.layers[0].layer_id.clone(),
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
