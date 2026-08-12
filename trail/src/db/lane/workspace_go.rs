use super::workspace_environment::{
    resolve_workspace_tool_executable, WorkspaceEnvironmentAdapter,
    WorkspaceEnvironmentAdapterMetadata, WorkspaceEnvironmentAdapterProposal,
    WorkspaceEnvironmentCacheAccess, WorkspaceEnvironmentCacheCommandBinding,
    WorkspaceEnvironmentCacheProtocol, WorkspaceEnvironmentCommand,
    WorkspaceEnvironmentCommandBinding, WorkspaceEnvironmentConstructionSeed,
    WorkspaceEnvironmentOutput, WorkspaceEnvironmentOutputPolicy, WorkspaceEnvironmentPlan,
    WorkspaceEnvironmentSandboxPolicy, WorkspaceEnvironmentToolCommandBinding,
};
use super::*;
use crate::ids::sha256_hex;

pub(crate) struct GoVendorAdapter;
pub(crate) struct GoWorkspaceVendorAdapter;

pub(crate) static GO_VENDOR_ADAPTER: GoVendorAdapter = GoVendorAdapter;
pub(crate) static GO_WORKSPACE_VENDOR_ADAPTER: GoWorkspaceVendorAdapter = GoWorkspaceVendorAdapter;

const MAX_GO_WORK_BYTES: u64 = 1024 * 1024;
const MAX_GO_WORK_MEMBERS: usize = 4096;

static GO_VENDOR_ADAPTER_METADATA: WorkspaceEnvironmentAdapterMetadata =
    WorkspaceEnvironmentAdapterMetadata {
        canonical_identity: "trail/go-vendor@1",
        namespace: "trail",
        name: "go-vendor",
        contract_major: 1,
        implementation_version: env!("CARGO_PKG_VERSION"),
        distribution_digest: "builtin:go-vendor-plan-v2",
        selectors: &["trail/go-vendor@1", "go-vendor", "go"],
        kind: "dependency",
        layer_adapter_name: "go-vendor",
        discovery_markers: &["go.mod"],
        supported_operating_systems: &["linux", "macos", "windows"],
        supported_architectures: &["aarch64", "x86_64"],
        stability: "experimental",
        description: "Single-module Go vendor tree with shared module and compiler caches",
    };

static GO_WORKSPACE_VENDOR_ADAPTER_METADATA: WorkspaceEnvironmentAdapterMetadata =
    WorkspaceEnvironmentAdapterMetadata {
        canonical_identity: "trail/go-vendor@2",
        namespace: "trail",
        name: "go-vendor-workspace",
        contract_major: 2,
        implementation_version: env!("CARGO_PKG_VERSION"),
        distribution_digest: "builtin:go-workspace-vendor-plan-v1",
        selectors: &["trail/go-vendor@2", "go-vendor-workspace", "go-work"],
        kind: "dependency",
        layer_adapter_name: "go-vendor",
        discovery_markers: &["go.work"],
        supported_operating_systems: &["linux", "macos", "windows"],
        supported_architectures: &["aarch64", "x86_64"],
        stability: "experimental",
        description: "Multi-module Go workspace vendor tree with a contained member graph",
    };

const GO_CACHE_COMMAND_BINDINGS: &[WorkspaceEnvironmentCacheCommandBinding] = &[
    WorkspaceEnvironmentCacheCommandBinding {
        cache_name: "module-store",
        environment: "GOMODCACHE",
        relative_path: "",
        required: true,
    },
    WorkspaceEnvironmentCacheCommandBinding {
        cache_name: "build-cache",
        environment: "GOCACHE",
        relative_path: "",
        required: true,
    },
];

const GO_COMMAND_BINDINGS: &[WorkspaceEnvironmentCommandBinding] = &[
    WorkspaceEnvironmentCommandBinding {
        environment: "GOWORK",
        value: "off",
    },
    WorkspaceEnvironmentCommandBinding {
        environment: "GOTOOLCHAIN",
        value: "local",
    },
    WorkspaceEnvironmentCommandBinding {
        environment: "GOFLAGS",
        value: "-mod=vendor -trimpath",
    },
];

const GO_WORKSPACE_COMMAND_BINDINGS: &[WorkspaceEnvironmentCommandBinding] = &[
    WorkspaceEnvironmentCommandBinding {
        environment: "GOTOOLCHAIN",
        value: "local",
    },
    WorkspaceEnvironmentCommandBinding {
        environment: "GOFLAGS",
        value: "-mod=vendor -trimpath",
    },
];

const GO_TOOL_COMMAND_BINDINGS: &[WorkspaceEnvironmentToolCommandBinding] =
    &[WorkspaceEnvironmentToolCommandBinding {
        programs: &["go"],
        environment: "TRAIL_GO",
        required: true,
        prepend_path: true,
    }];

impl WorkspaceEnvironmentAdapter for GoVendorAdapter {
    fn metadata(&self) -> &'static WorkspaceEnvironmentAdapterMetadata {
        &GO_VENDOR_ADAPTER_METADATA
    }

    fn component_id(&self, component_root: &str) -> Result<String> {
        let root = normalize_component_root(component_root)?;
        Ok(if root.is_empty() {
            "go-vendor".to_string()
        } else {
            format!("go-vendor:{root}")
        })
    }

    fn cache_command_bindings(&self) -> &'static [WorkspaceEnvironmentCacheCommandBinding] {
        GO_CACHE_COMMAND_BINDINGS
    }

    fn command_bindings(&self) -> &'static [WorkspaceEnvironmentCommandBinding] {
        GO_COMMAND_BINDINGS
    }

    fn tool_command_bindings(&self) -> &'static [WorkspaceEnvironmentToolCommandBinding] {
        GO_TOOL_COMMAND_BINDINGS
    }

    fn detect(&self, db: &Trail, source_root: &ObjectId, component_root: &str) -> Result<bool> {
        let root = normalize_component_root(component_root)?;
        Ok(db
            .root_file_entry(source_root, &join_repo_path(&root, "go.mod"))?
            .is_some())
    }

    fn propose(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<Option<WorkspaceEnvironmentAdapterProposal>> {
        let root = normalize_component_root(component_root)?;
        if db
            .root_file_entry(source_root, &join_repo_path(&root, "go.mod"))?
            .is_none()
        {
            return Ok(None);
        }
        if db
            .root_file_entry(source_root, &join_repo_path(&root, "go.work"))?
            .is_some()
            || go_component_is_workspace_member(db, source_root, &root)?
        {
            return Ok(None);
        }
        Ok(Some(WorkspaceEnvironmentAdapterProposal::ready()))
    }

    fn plan(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<WorkspaceEnvironmentPlan> {
        let component_root = normalize_component_root(component_root)?;
        let go_mod_path = join_repo_path(&component_root, "go.mod");
        let go_mod = db
            .root_file_entry(source_root, &go_mod_path)?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "Go component `{}` has no go.mod",
                    display_component_root(&component_root)
                ))
            })?;
        let go_sum_path = join_repo_path(&component_root, "go.sum");
        let go_sum = db.root_file_entry(source_root, &go_sum_path)?;
        let go_version = command_identity("go", &["version"])?;
        let go_tool = resolve_workspace_tool_executable("go")?;
        let implementation_version = env!("CARGO_PKG_VERSION").to_string();
        let distribution_digest = "builtin:go-vendor-plan-v2".to_string();
        let cache_compatibility = BTreeMap::from([
            ("go".to_string(), go_version.clone()),
            ("go_executable".to_string(), go_tool.identity.clone()),
            ("platform".to_string(), std::env::consts::OS.to_string()),
            (
                "architecture".to_string(),
                std::env::consts::ARCH.to_string(),
            ),
        ]);
        let module_cache = db.declare_workspace_environment_cache(
            self.identity(),
            "module-store",
            WorkspaceEnvironmentCacheProtocol::ContentStore,
            WorkspaceEnvironmentCacheAccess::ToolConcurrent,
            cache_compatibility.clone(),
        )?;
        let build_cache = db.declare_workspace_environment_cache(
            self.identity(),
            "build-cache",
            WorkspaceEnvironmentCacheProtocol::ContentStore,
            WorkspaceEnvironmentCacheAccess::ToolConcurrent,
            cache_compatibility,
        )?;
        let working_directory = if component_root.is_empty() {
            "project".to_string()
        } else {
            format!("project/{component_root}")
        };
        let mount_path = if component_root.is_empty() {
            "vendor".to_string()
        } else {
            format!("{component_root}/vendor")
        };
        let mut inputs = BTreeMap::from([
            ("source_root".to_string(), source_root.0.clone()),
            (go_mod_path, go_mod.content_hash),
            (
                "adapter_implementation".to_string(),
                implementation_version.clone(),
            ),
            (
                "adapter_distribution_digest".to_string(),
                distribution_digest.clone(),
            ),
            (
                "output_contract".to_string(),
                format!("immutable-seed-private:{mount_path}"),
            ),
            (
                "command_environment".to_string(),
                "GOMODCACHE=cache:module-store;GOCACHE=cache:build-cache;GOWORK=off;GOTOOLCHAIN=local;GOFLAGS=-mod=vendor -trimpath;TRAIL_GO=tool:go;PATH+=tool-dir:go"
                    .to_string(),
            ),
        ]);
        inputs.insert(
            go_sum_path,
            go_sum
                .map(|entry| entry.content_hash)
                .unwrap_or_else(|| "missing".to_string()),
        );
        let environment = BTreeMap::from([
            ("GOWORK".to_string(), "off".to_string()),
            ("GOTOOLCHAIN".to_string(), "local".to_string()),
            (
                "GOMODCACHE".to_string(),
                module_cache.storage_path.to_string_lossy().into_owned(),
            ),
            (
                "GOCACHE".to_string(),
                build_cache.storage_path.to_string_lossy().into_owned(),
            ),
        ]);
        Ok(WorkspaceEnvironmentPlan {
            component_id: self.component_id(&component_root)?,
            adapter_identity: self.identity().to_string(),
            adapter_version: 1,
            implementation_version,
            distribution_digest,
            kind: "dependency".to_string(),
            dependencies: Vec::new(),
            resolved_dependencies: Vec::new(),
            layer_key: WorkspaceLayerKeyV1 {
                kind: "dependency".to_string(),
                adapter: self.layer_adapter_name().to_string(),
                adapter_version: 1,
                inputs,
                tool_versions: BTreeMap::from([
                    ("go".to_string(), go_version),
                    ("go-executable".to_string(), go_tool.identity.clone()),
                ]),
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "source-root-go-toolchain-platform".to_string(),
                strategy: "go-mod-vendor-v1".to_string(),
            },
            inputs: Vec::new(),
            resolution_inputs: Vec::new(),
            construction_seed: Some(WorkspaceEnvironmentConstructionSeed {
                ignored_identity_inputs: BTreeSet::from([
                    "source_root".to_string(),
                    "host:adapter_identity_v3".to_string(),
                ]),
            }),
            source_projection: Some((source_root.clone(), "project".to_string())),
            pre_commands: Vec::new(),
            command: Some(WorkspaceEnvironmentCommand {
                program: "go".to_string(),
                resolved_program: go_tool.path,
                executable_identity: go_tool.identity,
                args: vec!["mod".to_string(), "vendor".to_string()],
                working_directory: working_directory.clone(),
                environment,
                remove_environment: Vec::new(),
                cache_names: vec![module_cache.name.clone(), build_cache.name.clone()],
            }),
            mounted_commands: Vec::new(),
            caches: vec![module_cache, build_cache],
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
            outputs: vec![WorkspaceEnvironmentOutput {
                name: "vendor".to_string(),
                output_path: format!("{working_directory}/vendor"),
                mount_path,
                policy: WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate,
                reuse: EnvironmentReuseMode::Exact,
                scope: EnvironmentSharingScope::Workspace,
                publish: EnvironmentPublicationTrigger::OnSync,
                gate: None,
                create_if_missing: true,
            }],
            stale_reason:
                "source root, Go module graph, Go toolchain, platform, or adapter policy changed"
                    .to_string(),
        })
    }
}

impl WorkspaceEnvironmentAdapter for GoWorkspaceVendorAdapter {
    fn metadata(&self) -> &'static WorkspaceEnvironmentAdapterMetadata {
        &GO_WORKSPACE_VENDOR_ADAPTER_METADATA
    }

    fn component_id(&self, component_root: &str) -> Result<String> {
        let root = normalize_component_root(component_root)?;
        Ok(if root.is_empty() {
            "go-vendor".to_string()
        } else {
            format!("go-vendor:{root}")
        })
    }

    fn cache_command_bindings(&self) -> &'static [WorkspaceEnvironmentCacheCommandBinding] {
        GO_CACHE_COMMAND_BINDINGS
    }

    fn command_bindings(&self) -> &'static [WorkspaceEnvironmentCommandBinding] {
        GO_WORKSPACE_COMMAND_BINDINGS
    }

    fn tool_command_bindings(&self) -> &'static [WorkspaceEnvironmentToolCommandBinding] {
        GO_TOOL_COMMAND_BINDINGS
    }

    fn detect(&self, db: &Trail, source_root: &ObjectId, component_root: &str) -> Result<bool> {
        let root = normalize_component_root(component_root)?;
        Ok(db
            .root_file_entry(source_root, &join_repo_path(&root, "go.work"))?
            .is_some())
    }

    fn propose(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<Option<WorkspaceEnvironmentAdapterProposal>> {
        let root = normalize_component_root(component_root)?;
        let Some(entry) = db.root_file_entry(source_root, &join_repo_path(&root, "go.work"))?
        else {
            return Ok(None);
        };
        let _ = load_go_workspace_graph(db, source_root, &root, &entry)?;
        Ok(Some(WorkspaceEnvironmentAdapterProposal::ready()))
    }

    fn plan(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<WorkspaceEnvironmentPlan> {
        let component_root = normalize_component_root(component_root)?;
        let go_work_path = join_repo_path(&component_root, "go.work");
        let go_work = db
            .root_file_entry(source_root, &go_work_path)?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "Go workspace component `{}` has no go.work",
                    display_component_root(&component_root)
                ))
            })?;
        let graph = load_go_workspace_graph(db, source_root, &component_root, &go_work)?;
        let go_version = command_identity("go", &["version"])?;
        let go_tool = resolve_workspace_tool_executable("go")?;
        let implementation_version = env!("CARGO_PKG_VERSION").to_string();
        let distribution_digest = "builtin:go-workspace-vendor-plan-v1".to_string();
        let cache_compatibility = BTreeMap::from([
            ("go".to_string(), go_version.clone()),
            ("go_executable".to_string(), go_tool.identity.clone()),
            ("platform".to_string(), std::env::consts::OS.to_string()),
            (
                "architecture".to_string(),
                std::env::consts::ARCH.to_string(),
            ),
        ]);
        let module_cache = db.declare_workspace_environment_cache(
            self.identity(),
            "module-store",
            WorkspaceEnvironmentCacheProtocol::ContentStore,
            WorkspaceEnvironmentCacheAccess::ToolConcurrent,
            cache_compatibility.clone(),
        )?;
        let build_cache = db.declare_workspace_environment_cache(
            self.identity(),
            "build-cache",
            WorkspaceEnvironmentCacheProtocol::ContentStore,
            WorkspaceEnvironmentCacheAccess::ToolConcurrent,
            cache_compatibility,
        )?;
        let working_directory = if component_root.is_empty() {
            "project".to_string()
        } else {
            format!("project/{component_root}")
        };
        let mount_path = join_repo_path(&component_root, "vendor");
        let mut inputs = BTreeMap::from([
            ("source_root".to_string(), source_root.0.clone()),
            (go_work_path.clone(), go_work.content_hash),
            (
                "adapter_implementation".to_string(),
                implementation_version.clone(),
            ),
            (
                "adapter_distribution_digest".to_string(),
                distribution_digest.clone(),
            ),
            (
                "workspace_members".to_string(),
                go_workspace_members_digest(&graph.members),
            ),
            (
                "output_contract".to_string(),
                format!("immutable-seed-private:{mount_path}"),
            ),
            (
                "command_environment".to_string(),
                "GOMODCACHE=cache:module-store;GOCACHE=cache:build-cache;GOTOOLCHAIN=local;GOFLAGS=-mod=vendor -trimpath;TRAIL_GO=tool:go;PATH+=tool-dir:go"
                    .to_string(),
            ),
        ]);
        let go_work_sum_path = join_repo_path(&component_root, "go.work.sum");
        inputs.insert(
            go_work_sum_path.clone(),
            db.root_file_entry(source_root, &go_work_sum_path)?
                .map(|entry| entry.content_hash)
                .unwrap_or_else(|| "missing".to_string()),
        );
        for member in &graph.members {
            let go_mod_path = join_repo_path(member, "go.mod");
            let go_mod = db
                .root_file_entry(source_root, &go_mod_path)?
                .ok_or_else(|| {
                    Error::InvalidInput(format!("Go workspace member `{member}` has no go.mod"))
                })?;
            inputs.insert(go_mod_path, go_mod.content_hash);
            let go_sum_path = join_repo_path(member, "go.sum");
            inputs.insert(
                go_sum_path.clone(),
                db.root_file_entry(source_root, &go_sum_path)?
                    .map(|entry| entry.content_hash)
                    .unwrap_or_else(|| "missing".to_string()),
            );
        }
        let environment = BTreeMap::from([
            ("GOTOOLCHAIN".to_string(), "local".to_string()),
            (
                "GOMODCACHE".to_string(),
                module_cache.storage_path.to_string_lossy().into_owned(),
            ),
            (
                "GOCACHE".to_string(),
                build_cache.storage_path.to_string_lossy().into_owned(),
            ),
        ]);
        Ok(WorkspaceEnvironmentPlan {
            component_id: self.component_id(&component_root)?,
            adapter_identity: self.identity().to_string(),
            adapter_version: 2,
            implementation_version,
            distribution_digest,
            kind: "dependency".to_string(),
            dependencies: Vec::new(),
            resolved_dependencies: Vec::new(),
            layer_key: WorkspaceLayerKeyV1 {
                kind: "dependency".to_string(),
                adapter: self.layer_adapter_name().to_string(),
                adapter_version: 2,
                inputs,
                tool_versions: BTreeMap::from([
                    ("go".to_string(), go_version),
                    ("go-executable".to_string(), go_tool.identity.clone()),
                ]),
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "source-root-go-toolchain-platform".to_string(),
                strategy: "go-work-vendor-v2".to_string(),
            },
            inputs: Vec::new(),
            resolution_inputs: Vec::new(),
            construction_seed: Some(WorkspaceEnvironmentConstructionSeed {
                ignored_identity_inputs: BTreeSet::from([
                    "source_root".to_string(),
                    "host:adapter_identity_v3".to_string(),
                ]),
            }),
            source_projection: Some((source_root.clone(), "project".to_string())),
            pre_commands: Vec::new(),
            command: Some(WorkspaceEnvironmentCommand {
                program: "go".to_string(),
                resolved_program: go_tool.path,
                executable_identity: go_tool.identity,
                args: vec!["work".to_string(), "vendor".to_string()],
                working_directory: working_directory.clone(),
                environment,
                remove_environment: vec!["GOWORK".to_string(), "GOFLAGS".to_string()],
                cache_names: vec![module_cache.name.clone(), build_cache.name.clone()],
            }),
            mounted_commands: Vec::new(),
            caches: vec![module_cache, build_cache],
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
            outputs: vec![WorkspaceEnvironmentOutput {
                name: "vendor".to_string(),
                output_path: format!("{working_directory}/vendor"),
                mount_path,
                policy: WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate,
                reuse: EnvironmentReuseMode::Exact,
                scope: EnvironmentSharingScope::Workspace,
                publish: EnvironmentPublicationTrigger::OnSync,
                gate: None,
                create_if_missing: true,
            }],
            stale_reason:
                "source root, Go workspace graph, Go toolchain, platform, or adapter policy changed"
                    .to_string(),
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct GoWorkspaceGraph {
    members: Vec<String>,
}

fn go_workspace_members_digest(members: &[String]) -> String {
    let mut framed = Vec::new();
    for member in members {
        framed.extend_from_slice(&(member.len() as u64).to_be_bytes());
        framed.extend_from_slice(member.as_bytes());
    }
    sha256_hex(&framed)
}

fn load_go_workspace_graph(
    db: &Trail,
    source_root: &ObjectId,
    component_root: &str,
    go_work: &FileEntry,
) -> Result<GoWorkspaceGraph> {
    if go_work.size_bytes > MAX_GO_WORK_BYTES {
        return Err(Error::InvalidInput(format!(
            "Go workspace `{}` is {} bytes; maximum is {MAX_GO_WORK_BYTES}",
            join_repo_path(component_root, "go.work"),
            go_work.size_bytes
        )));
    }
    let bytes = db.materialize_entry_bytes(go_work)?;
    parse_go_workspace_graph(&bytes, component_root, |member| {
        Ok(db
            .root_file_entry(source_root, &join_repo_path(member, "go.mod"))?
            .is_some())
    })
}

fn parse_go_workspace_graph(
    bytes: &[u8],
    component_root: &str,
    mut member_exists: impl FnMut(&str) -> Result<bool>,
) -> Result<GoWorkspaceGraph> {
    if bytes.len() as u64 > MAX_GO_WORK_BYTES {
        return Err(Error::InvalidInput(format!(
            "Go workspace is {} bytes; maximum is {MAX_GO_WORK_BYTES}",
            bytes.len()
        )));
    }
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::InvalidInput("Go workspace go.work is not UTF-8".to_string()))?;
    let mut in_block_comment = false;
    let mut in_use_block = false;
    let mut in_replace_block = false;
    let mut members = BTreeSet::new();
    for (index, raw_line) in text.lines().enumerate() {
        let line = strip_go_work_comments(raw_line, &mut in_block_comment)?;
        let tokens = go_work_line_tokens(&line)?;
        if tokens.is_empty() {
            continue;
        }
        let candidate = if in_use_block {
            if tokens.as_slice() == [")"] {
                in_use_block = false;
                continue;
            }
            if tokens.len() != 1 {
                return Err(Error::InvalidInput(format!(
                    "Go workspace use block line {} must contain exactly one module directory",
                    index + 1
                )));
            }
            Some(tokens[0].as_str())
        } else if in_replace_block {
            if tokens.as_slice() == [")"] {
                in_replace_block = false;
                continue;
            }
            validate_go_workspace_replacement(
                &tokens,
                component_root,
                &mut member_exists,
                index + 1,
            )?;
            None
        } else if tokens[0] == "use" {
            match tokens.as_slice() {
                [_, token] if token == "(" => {
                    in_use_block = true;
                    None
                }
                [_, member] => Some(member.as_str()),
                _ => {
                    return Err(Error::InvalidInput(format!(
                        "Go workspace use directive on line {} is malformed",
                        index + 1
                    )));
                }
            }
        } else if tokens[0] == "replace" {
            match tokens.as_slice() {
                [_, token] if token == "(" => {
                    in_replace_block = true;
                }
                [_, replacement @ ..] => validate_go_workspace_replacement(
                    replacement,
                    component_root,
                    &mut member_exists,
                    index + 1,
                )?,
                _ => unreachable!("replace token is present"),
            }
            None
        } else {
            None
        };
        let Some(candidate) = candidate else {
            continue;
        };
        let member = resolve_go_workspace_member(component_root, candidate)?;
        if !member_exists(&member)? {
            return Err(Error::InvalidInput(format!(
                "Go workspace member `{candidate}` has no contained go.mod at `{}`",
                join_repo_path(&member, "go.mod")
            )));
        }
        if !members.insert(member.clone()) {
            return Err(Error::InvalidInput(format!(
                "Go workspace contains duplicate member `{member}`"
            )));
        }
        if members.len() > MAX_GO_WORK_MEMBERS {
            return Err(Error::InvalidInput(format!(
                "Go workspace has more than {MAX_GO_WORK_MEMBERS} members"
            )));
        }
    }
    if in_block_comment {
        return Err(Error::InvalidInput(
            "Go workspace contains an unterminated block comment".to_string(),
        ));
    }
    if in_use_block {
        return Err(Error::InvalidInput(
            "Go workspace contains an unterminated use block".to_string(),
        ));
    }
    if in_replace_block {
        return Err(Error::InvalidInput(
            "Go workspace contains an unterminated replace block".to_string(),
        ));
    }
    if members.is_empty() {
        return Err(Error::InvalidInput(
            "Go workspace must declare at least one use member".to_string(),
        ));
    }
    Ok(GoWorkspaceGraph {
        members: members.into_iter().collect(),
    })
}

fn validate_go_workspace_replacement(
    tokens: &[String],
    component_root: &str,
    member_exists: &mut impl FnMut(&str) -> Result<bool>,
    line: usize,
) -> Result<()> {
    let arrows = tokens
        .iter()
        .enumerate()
        .filter_map(|(index, token)| (token == "=>").then_some(index))
        .collect::<Vec<_>>();
    if arrows.len() != 1 {
        return Err(Error::InvalidInput(format!(
            "Go workspace replace directive on line {line} must contain one `=>`"
        )));
    }
    let arrow = arrows[0];
    let left = &tokens[..arrow];
    let right = &tokens[arrow + 1..];
    if !(1..=2).contains(&left.len()) || !(1..=2).contains(&right.len()) {
        return Err(Error::InvalidInput(format!(
            "Go workspace replace directive on line {line} is malformed"
        )));
    }
    if right.len() == 2 {
        return Ok(());
    }
    let target = &right[0];
    if !target.starts_with('.') && !target.starts_with('/') && !target.contains('\\') {
        return Err(Error::InvalidInput(format!(
            "Go workspace replacement `{target}` on line {line} must include a version or use a contained relative directory"
        )));
    }
    let resolved = resolve_go_workspace_member(component_root, target)?;
    if !member_exists(&resolved)? {
        return Err(Error::InvalidInput(format!(
            "Go workspace replacement `{target}` has no contained go.mod at `{}`",
            join_repo_path(&resolved, "go.mod")
        )));
    }
    Ok(())
}

fn resolve_go_workspace_member(component_root: &str, member: &str) -> Result<String> {
    let member = if member == "." || member == "./" {
        String::new()
    } else {
        normalize_relative_path(member).map_err(|error| {
            Error::InvalidInput(format!(
                "Go workspace member `{member}` must stay inside its component root: {error}"
            ))
        })?
    };
    match (component_root.is_empty(), member.is_empty()) {
        (true, true) => Ok(String::new()),
        (true, false) => Ok(member),
        (false, true) => Ok(component_root.to_string()),
        (false, false) => normalize_relative_path(&format!("{component_root}/{member}")),
    }
}

fn go_component_is_workspace_member(
    db: &Trail,
    source_root: &ObjectId,
    component_root: &str,
) -> Result<bool> {
    if component_root.is_empty() {
        return Ok(false);
    }
    let segments = component_root.split('/').collect::<Vec<_>>();
    for depth in (0..segments.len()).rev() {
        let ancestor = segments[..depth].join("/");
        let Some(go_work) =
            db.root_file_entry(source_root, &join_repo_path(&ancestor, "go.work"))?
        else {
            continue;
        };
        let graph = load_go_workspace_graph(db, source_root, &ancestor, &go_work)?;
        if graph
            .members
            .binary_search(&component_root.to_string())
            .is_ok()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn strip_go_work_comments(line: &str, in_block: &mut bool) -> Result<String> {
    let bytes = line.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    let mut quote = None;
    while index < bytes.len() {
        if *in_block {
            if bytes[index..].starts_with(b"*/") {
                *in_block = false;
                index += 2;
            } else {
                index += 1;
            }
            continue;
        }
        if let Some(delimiter) = quote {
            output.push(bytes[index]);
            if bytes[index] == delimiter {
                quote = None;
            } else if delimiter == b'"' && bytes[index] == b'\\' {
                index += 1;
                if index >= bytes.len() {
                    return Err(Error::InvalidInput(
                        "Go workspace contains an unterminated quoted string".to_string(),
                    ));
                }
                output.push(bytes[index]);
            }
            index += 1;
            continue;
        }
        if bytes[index..].starts_with(b"//") {
            break;
        }
        if bytes[index..].starts_with(b"/*") {
            *in_block = true;
            index += 2;
            continue;
        }
        if matches!(bytes[index], b'"' | b'`') {
            quote = Some(bytes[index]);
        }
        output.push(bytes[index]);
        index += 1;
    }
    if quote.is_some() {
        return Err(Error::InvalidInput(
            "Go workspace contains an unterminated quoted string".to_string(),
        ));
    }
    String::from_utf8(output)
        .map_err(|_| Error::InvalidInput("Go workspace go.work is not UTF-8".to_string()))
}

fn go_work_line_tokens(line: &str) -> Result<Vec<String>> {
    let mut tokens = Vec::new();
    let mut chars = line.char_indices().peekable();
    while let Some((_, ch)) = chars.peek().copied() {
        if ch.is_whitespace() {
            chars.next();
            continue;
        }
        if matches!(ch, '(' | ')') {
            chars.next();
            tokens.push(ch.to_string());
            continue;
        }
        if matches!(ch, '"' | '`') {
            let delimiter = ch;
            chars.next();
            let mut value = String::new();
            let mut closed = false;
            while let Some((_, current)) = chars.next() {
                if current == delimiter {
                    closed = true;
                    break;
                }
                if delimiter == '"' && current == '\\' {
                    let Some((_, escaped)) = chars.next() else {
                        break;
                    };
                    match escaped {
                        '\\' | '"' => value.push(escaped),
                        'n' => value.push('\n'),
                        'r' => value.push('\r'),
                        't' => value.push('\t'),
                        _ => {
                            return Err(Error::InvalidInput(format!(
                                "Go workspace path uses unsupported escape `\\{escaped}`"
                            )));
                        }
                    }
                } else {
                    value.push(current);
                }
            }
            if !closed {
                return Err(Error::InvalidInput(
                    "Go workspace contains an unterminated quoted path".to_string(),
                ));
            }
            tokens.push(value);
            continue;
        }
        let mut value = String::new();
        while let Some((_, current)) = chars.peek().copied() {
            if current.is_whitespace() || matches!(current, '(' | ')') {
                break;
            }
            value.push(current);
            chars.next();
        }
        tokens.push(value);
    }
    Ok(tokens)
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

fn command_identity(program: &str, args: &[&str]) -> Result<String> {
    let output = Command::new(program).args(args).output().map_err(|err| {
        Error::InvalidInput(format!("required tool `{program}` is unavailable: {err}"))
    })?;
    if !output.status.success() {
        return Err(Error::InvalidInput(format!(
            "`{program} {}` failed with {}",
            args.join(" "),
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;

    #[test]
    fn go_workspace_graph_is_contained_bounded_and_deterministic() {
        let graph = parse_go_workspace_graph(
            br#"
                go 1.22
                // paths may be quoted and comments are ignored
                use (
                    "./zeta"
                    ./alpha /* retained member */
                )
                replace example.com/old => ./alpha
            "#,
            "nested",
            |member| Ok(matches!(member, "nested/alpha" | "nested/zeta")),
        )
        .unwrap();
        assert_eq!(graph.members, vec!["nested/alpha", "nested/zeta"]);

        let duplicate =
            parse_go_workspace_graph(b"go 1.22\nuse (\n./member\nmember\n)\n", "", |_| Ok(true))
                .unwrap_err();
        assert!(duplicate.to_string().contains("duplicate member"));

        let traversal =
            parse_go_workspace_graph(b"go 1.22\nuse ../outside\n", "nested", |_| Ok(true))
                .unwrap_err();
        assert!(traversal.to_string().contains("must stay inside"));

        let missing =
            parse_go_workspace_graph(b"go 1.22\nuse ./missing\n", "", |_| Ok(false)).unwrap_err();
        assert!(missing.to_string().contains("has no contained go.mod"));

        let too_large = vec![b'x'; MAX_GO_WORK_BYTES as usize + 1];
        let oversized = parse_go_workspace_graph(&too_large, "", |_| Ok(true)).unwrap_err();
        assert!(oversized.to_string().contains("maximum"));

        let escaping_replacement = parse_go_workspace_graph(
            b"go 1.22\nuse ./member\nreplace example.com/old => ../outside\n",
            "nested",
            |member| Ok(member == "nested/member"),
        )
        .unwrap_err();
        assert!(escaping_replacement
            .to_string()
            .contains("must stay inside"));

        let mut too_many_members = String::from("go 1.22\nuse (\n");
        for index in 0..=MAX_GO_WORK_MEMBERS {
            too_many_members.push_str(&format!("./member-{index}\n"));
        }
        too_many_members.push_str(")\n");
        let over_limit =
            parse_go_workspace_graph(too_many_members.as_bytes(), "", |_| Ok(true)).unwrap_err();
        assert!(over_limit.to_string().contains("more than"));

        let unterminated =
            parse_go_workspace_graph(b"go 1.22\nuse (\n./member\n", "", |_| Ok(true)).unwrap_err();
        assert!(unterminated.to_string().contains("unterminated use block"));
    }

    #[cfg(unix)]
    #[test]
    fn go_workspace_symlink_member_never_enters_the_pinned_graph() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        fs::write(
            external.path().join("go.mod"),
            "module example.com/external\n\ngo 1.22\n",
        )
        .unwrap();
        fs::write(workspace.path().join("go.work"), "go 1.22\nuse ./linked\n").unwrap();
        symlink(external.path(), workspace.path().join("linked")).unwrap();

        let initialized = Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false);
        if initialized.is_err() {
            return;
        }
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "symlink-workspace",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let error = db
            .discover_workspace_environment("symlink-workspace", None)
            .unwrap_err();
        assert!(error.to_string().contains("has no contained go.mod"));
    }

    #[test]
    fn go_workspace_is_one_graph_component_and_constructs_vendor_output() {
        if command_identity("go", &["version"]).is_err() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("app")).unwrap();
        fs::create_dir_all(workspace.path().join("lib")).unwrap();
        fs::write(
            workspace.path().join("go.work"),
            "go 1.22\n\nuse (\n\t./app\n\t./lib\n)\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("app/go.mod"),
            "module example.com/app\n\ngo 1.22\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("app/main.go"),
            "package main\nfunc main() {}\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("lib/go.mod"),
            "module example.com/lib\n\ngo 1.22\n",
        )
        .unwrap();
        fs::write(workspace.path().join("lib/lib.go"), "package lib\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "go-workspace",
            Some("main"),
            mode,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();

        let discovery = db
            .discover_workspace_environment("go-workspace", None)
            .unwrap();
        assert_eq!(discovery.components.len(), 1);
        assert_eq!(discovery.components[0].component_id, "go-vendor");
        assert_eq!(
            discovery.components[0].adapter_identity,
            "trail/go-vendor@2"
        );
        let plan = GO_WORKSPACE_VENDOR_ADAPTER
            .plan(&db, &discovery.source_root, "")
            .unwrap();
        assert_eq!(plan.layer_key.strategy, "go-work-vendor-v2");
        assert_eq!(
            plan.command.as_ref().unwrap().args,
            vec!["work".to_string(), "vendor".to_string()]
        );
        assert_eq!(
            plan.layer_key.inputs["workspace_members"],
            go_workspace_members_digest(&["app".to_string(), "lib".to_string()])
        );

        let synced = db
            .sync_workspace_environment("go-workspace", "trail/go-vendor@2", None)
            .unwrap();
        assert!(Path::new(&synced.storage_path)
            .join("modules.txt")
            .is_file());
        let environment = db
            .lane_workspace_environment("go-workspace")
            .unwrap()
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        assert!(!environment.contains_key("GOWORK"));
        assert_eq!(environment["GOTOOLCHAIN"], "local");
        assert_eq!(environment["GOFLAGS"], "-mod=vendor -trimpath");
    }

    #[test]
    fn go_adapter_vendors_once_and_reuses_the_immutable_tree_across_lanes() {
        if command_identity("go", &["version"]).is_err() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("shared")).unwrap();
        fs::create_dir_all(workspace.path().join("tools")).unwrap();
        fs::write(
            workspace.path().join("go.mod"),
            "module example.test/app\n\ngo 1.22\n\nrequire example.test/shared v0.0.0\nreplace example.test/shared => ./shared\n",
        )
        .unwrap();
        fs::write(workspace.path().join("go.sum"), "").unwrap();
        fs::write(
            workspace.path().join("main.go"),
            "package main\nimport _ \"example.test/shared\"\nfunc main() {}\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("shared/go.mod"),
            "module example.test/shared\n\ngo 1.22\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("shared/shared.go"),
            "package shared\nconst Value = 42\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("tools/go.mod"),
            "module example.test/tools\n\ngo 1.22\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("tools/main.go"),
            "package main\nfunc main() {}\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let source_root = db.resolve_branch_ref("main").unwrap().root_id;
        let raw_plan = GO_VENDOR_ADAPTER.plan(&db, &source_root, "").unwrap();
        let contract_digest =
            super::workspace_environment::workspace_environment_artifact_contract_digest(&raw_plan)
                .unwrap();
        let identity = super::workspace_environment::workspace_environment_identity_contract_v3(
            &raw_plan,
            contract_digest.clone(),
        )
        .unwrap();
        assert!(identity.source_closure_complete);
        assert!(identity.portability_certified);
        assert_eq!(identity.trust_scope, "builtin");
        assert_eq!(identity.semantic_identities.len(), 3);
        assert!(identity
            .semantic_identities
            .contains_key("performance_cache:module-store"));
        assert!(identity
            .semantic_identities
            .contains_key("performance_cache:build-cache"));

        let mut relocated_plan = raw_plan.clone();
        for cache in &mut relocated_plan.caches {
            let old_path = cache.storage_path.to_string_lossy().into_owned();
            cache.storage_path = PathBuf::from(format!("relocated-cache-{}", cache.name));
            let new_path = cache.storage_path.to_string_lossy().into_owned();
            for command in relocated_plan.command.iter_mut() {
                for value in command.environment.values_mut() {
                    if value == &old_path {
                        *value = new_path.clone();
                    }
                }
            }
        }
        assert_eq!(
            super::workspace_environment::workspace_environment_artifact_contract_digest(
                &relocated_plan,
            )
            .unwrap(),
            contract_digest,
            "host cache locations are execution bindings, not artifact identity"
        );
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        for lane in ["go-one", "go-two", "go-all"] {
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
        let first = db
            .sync_workspace_environment("go-one", "auto", None)
            .unwrap();
        let second = db
            .sync_workspace_environment("go-two", "trail/go-vendor@1", None)
            .unwrap();
        assert_eq!(first.layer_id, second.layer_id);
        assert!(Path::new(&first.storage_path)
            .join("example.test/shared/shared.go")
            .is_file());
        let status = db.environment_component_status("go-two").unwrap();
        assert_eq!(status[0].component.component_id, "go-vendor");
        assert_eq!(status[0].adapter.name, "go-vendor");
        assert_eq!(status[0].status, "ready");
        let go_one_environment = db
            .lane_workspace_environment("go-one")
            .unwrap()
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let generation = db.active_environment_generation("go-one").unwrap().unwrap();
        let caches = generation.components[0]
            .caches
            .iter()
            .map(|cache| (cache.name.as_str(), cache.namespace_id.as_str()))
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            Path::new(&go_one_environment["GOMODCACHE"]),
            db.db_dir
                .join("cache/namespaces")
                .join(caches["module-store"])
        );
        assert_eq!(
            Path::new(&go_one_environment["GOCACHE"]),
            db.db_dir
                .join("cache/namespaces")
                .join(caches["build-cache"])
        );
        assert_eq!(go_one_environment["GOWORK"], "off");
        assert_eq!(go_one_environment["GOTOOLCHAIN"], "local");
        assert_eq!(go_one_environment["GOFLAGS"], "-mod=vendor -trimpath");
        assert!(Path::new(&go_one_environment["TRAIL_GO"]).is_absolute());
        assert_eq!(
            std::env::split_paths(OsStr::new(&go_one_environment["PATH"]))
                .next()
                .unwrap(),
            Path::new(&go_one_environment["TRAIL_GO"]).parent().unwrap()
        );

        let all = db.sync_all_workspace_environments("go-all", None).unwrap();
        assert_eq!(all.generation.generation_sequence, 1);
        assert_eq!(all.layers.len(), 3);
        assert_eq!(
            all.generation
                .components
                .iter()
                .map(|component| component.component_id.as_str())
                .collect::<Vec<_>>(),
            vec!["go-vendor", "go-vendor:shared", "go-vendor:tools"]
        );

        db.conn
            .execute_batch(
                "CREATE TRIGGER fail_generation_activation
                 BEFORE INSERT ON environment_generations
                 BEGIN
                     SELECT RAISE(ABORT, 'injected generation activation failure');
                 END;",
            )
            .unwrap();
        let activation_error = db
            .sync_workspace_environment("go-one", "trail/go-vendor@1", Some("tools"))
            .unwrap_err();
        assert!(activation_error.to_string().contains("injected generation"));
        let unchanged = db.active_environment_generation("go-one").unwrap().unwrap();
        assert_eq!(unchanged.generation_sequence, 1);
        assert_eq!(unchanged.components.len(), 1);
        let view = db.lane_workspace_view("go-one").unwrap().unwrap();
        let tools_binding = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM environment_component_bindings WHERE view_id = ?1 AND component_id = 'go-vendor:tools'",
                params![view.view_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(tools_binding, 0);
        db.conn
            .execute_batch("DROP TRIGGER fail_generation_activation")
            .unwrap();
        db.sync_workspace_environment("go-one", "trail/go-vendor@1", Some("tools"))
            .unwrap();
        let generation = db.active_environment_generation("go-one").unwrap().unwrap();
        assert_eq!(generation.generation_sequence, 2);
        assert_eq!(generation.components.len(), 2);
        assert_eq!(
            generation
                .components
                .iter()
                .map(|component| component.component_id.as_str())
                .collect::<Vec<_>>(),
            vec!["go-vendor", "go-vendor:tools"]
        );
        let predecessor = generation.predecessor_generation_id.unwrap();
        let command_environment = db.lane_workspace_environment("go-one").unwrap();
        assert_eq!(
            command_environment
                .iter()
                .find(|(name, _)| name == "TRAIL_ENVIRONMENT_GENERATION")
                .map(|(_, value)| value.as_str()),
            Some(generation.generation_id.as_str())
        );
        let predecessor_state = db
            .conn
            .query_row(
                "SELECT state FROM environment_generations WHERE generation_id = ?1",
                params![predecessor],
                |row| row.get::<_, String>(0),
            )
            .unwrap();
        assert_eq!(predecessor_state, "retired");
    }
}
