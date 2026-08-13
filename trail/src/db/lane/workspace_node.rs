use super::workspace_environment::{
    resolve_workspace_tool_executable, WorkspaceEnvironmentAdapter,
    WorkspaceEnvironmentAdapterMetadata, WorkspaceEnvironmentAdapterProposal,
    WorkspaceEnvironmentCacheAccess, WorkspaceEnvironmentCacheCommandBinding,
    WorkspaceEnvironmentCacheProtocol, WorkspaceEnvironmentCommand, WorkspaceEnvironmentInput,
    WorkspaceEnvironmentOutput, WorkspaceEnvironmentOutputCommandBinding,
    WorkspaceEnvironmentOutputPolicy, WorkspaceEnvironmentPlan,
    WorkspaceEnvironmentResolutionInput, WorkspaceEnvironmentSandboxPolicy,
    WorkspaceEnvironmentToolCommandBinding,
};
use super::*;
use crate::ids::sha256_hex;

pub(crate) struct NodeWorkspaceAdapter;

pub(crate) static NODE_WORKSPACE_ADAPTER: NodeWorkspaceAdapter = NodeWorkspaceAdapter;

static NODE_WORKSPACE_ADAPTER_METADATA: WorkspaceEnvironmentAdapterMetadata =
    WorkspaceEnvironmentAdapterMetadata {
        canonical_identity: "trail/node@1",
        namespace: "trail",
        name: "node",
        contract_major: 1,
        implementation_version: env!("CARGO_PKG_VERSION"),
        distribution_digest: "builtin:node-plan-v3",
        selectors: &["trail/node@1", "node"],
        kind: "dependency",
        layer_adapter_name: "node",
        discovery_markers: &["package.json"],
        supported_operating_systems: &["linux", "macos", "windows"],
        supported_architectures: &["aarch64", "x86_64"],
        stability: "stable",
        description:
            "Frozen npm, pnpm, Yarn, or Bun dependency tree with a private writable lane upper",
    };

const NODE_CACHE_COMMAND_BINDINGS: &[WorkspaceEnvironmentCacheCommandBinding] = &[
    WorkspaceEnvironmentCacheCommandBinding {
        cache_name: "package-manager",
        environment: "npm_config_cache",
        relative_path: "npm",
        required: true,
    },
    WorkspaceEnvironmentCacheCommandBinding {
        cache_name: "package-manager",
        environment: "PNPM_HOME",
        relative_path: "pnpm-home",
        required: true,
    },
    WorkspaceEnvironmentCacheCommandBinding {
        cache_name: "package-manager",
        environment: "PNPM_STORE_DIR",
        relative_path: "pnpm-store",
        required: true,
    },
    WorkspaceEnvironmentCacheCommandBinding {
        cache_name: "package-manager",
        environment: "YARN_CACHE_FOLDER",
        relative_path: "yarn",
        required: true,
    },
    WorkspaceEnvironmentCacheCommandBinding {
        cache_name: "package-manager",
        environment: "BUN_INSTALL_CACHE_DIR",
        relative_path: "bun",
        required: true,
    },
];

const NODE_TOOL_COMMAND_BINDINGS: &[WorkspaceEnvironmentToolCommandBinding] = &[
    WorkspaceEnvironmentToolCommandBinding {
        programs: &["node"],
        environment: "TRAIL_NODE",
        required: true,
        prepend_path: true,
    },
    WorkspaceEnvironmentToolCommandBinding {
        programs: &["npm"],
        environment: "TRAIL_NPM",
        required: false,
        prepend_path: true,
    },
    WorkspaceEnvironmentToolCommandBinding {
        programs: &["pnpm"],
        environment: "TRAIL_PNPM",
        required: false,
        prepend_path: true,
    },
    WorkspaceEnvironmentToolCommandBinding {
        programs: &["yarn"],
        environment: "TRAIL_YARN",
        required: false,
        prepend_path: true,
    },
    WorkspaceEnvironmentToolCommandBinding {
        programs: &["bun"],
        environment: "TRAIL_BUN",
        required: false,
        prepend_path: true,
    },
];

const NODE_OUTPUT_COMMAND_BINDINGS: &[WorkspaceEnvironmentOutputCommandBinding] = &[
    WorkspaceEnvironmentOutputCommandBinding {
        output_name: "modules",
        environment: Some("TRAIL_NODE_MODULES"),
        relative_path: "",
        direct: true,
        prepend_path: false,
        required: true,
    },
    WorkspaceEnvironmentOutputCommandBinding {
        output_name: "modules",
        environment: Some("NODE_PATH"),
        relative_path: "",
        direct: true,
        prepend_path: false,
        required: true,
    },
    WorkspaceEnvironmentOutputCommandBinding {
        output_name: "modules",
        environment: None,
        relative_path: ".bin",
        direct: true,
        prepend_path: true,
        required: true,
    },
];

const NODE_LIFECYCLE_POLICY_FILE: &str = "trail-node-lifecycle.json";
const MAX_NODE_LIFECYCLE_SOURCE_FILES: usize = 4_096;
const MAX_NODE_LIFECYCLE_SOURCE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeLifecyclePolicyV1 {
    version: u32,
    manager: String,
    lock_sha256: String,
    packages: Vec<String>,
    scripts: Vec<NodeLifecycleScriptV1>,
    capabilities: NodeLifecycleCapabilitiesV1,
    outputs: Vec<String>,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeLifecycleScriptV1 {
    package: String,
    phase: String,
}

#[derive(Clone, Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeLifecycleCapabilitiesV1 {
    network: String,
    native_toolchain: bool,
}

#[derive(Clone, Debug)]
struct ApprovedNodeLifecycle {
    path: String,
    entry: FileEntry,
    digest: String,
    script: String,
    outputs: Vec<String>,
}

impl WorkspaceEnvironmentAdapter for NodeWorkspaceAdapter {
    fn metadata(&self) -> &'static WorkspaceEnvironmentAdapterMetadata {
        &NODE_WORKSPACE_ADAPTER_METADATA
    }

    fn component_id(&self, component_root: &str) -> Result<String> {
        let root = normalize_package_root(component_root)?;
        Ok(if root.is_empty() {
            "node".to_string()
        } else {
            format!("node:{root}")
        })
    }

    fn cache_command_bindings(&self) -> &'static [WorkspaceEnvironmentCacheCommandBinding] {
        NODE_CACHE_COMMAND_BINDINGS
    }

    fn tool_command_bindings(&self) -> &'static [WorkspaceEnvironmentToolCommandBinding] {
        NODE_TOOL_COMMAND_BINDINGS
    }

    fn output_command_bindings(&self) -> &'static [WorkspaceEnvironmentOutputCommandBinding] {
        NODE_OUTPUT_COMMAND_BINDINGS
    }

    fn detect(&self, db: &Trail, source_root: &ObjectId, component_root: &str) -> Result<bool> {
        let root = normalize_package_root(component_root)?;
        Ok(db
            .root_file_entry(source_root, &join_repo_path(&root, "package.json"))?
            .is_some())
    }

    fn propose(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<Option<WorkspaceEnvironmentAdapterProposal>> {
        let root = normalize_package_root(component_root)?;
        if db
            .root_file_entry(source_root, &join_repo_path(&root, "package.json"))?
            .is_none()
        {
            return Ok(None);
        }
        if node_component_is_descendant_of_locked_workspace(db, source_root, &root)? {
            return Ok(None);
        }
        let package_path = join_repo_path(&root, "package.json");
        let package_entry = db
            .root_file_entry(source_root, &package_path)?
            .ok_or_else(|| Error::Corrupt(format!("Node manifest `{package_path}` disappeared")))?;
        let package_bytes = db.materialize_entry_bytes(&package_entry)?;
        let package: serde_json::Value =
            serde_json::from_slice(&package_bytes).map_err(|error| {
                Error::InvalidInput(format!(
                    "Node manifest `{package_path}` is malformed JSON: {error}"
                ))
            })?;
        if let Err(error) = validate_node_modules_layout(db, source_root, &root, &package) {
            return Ok(Some(WorkspaceEnvironmentAdapterProposal::blocked(
                EnvironmentProposalReasonReport {
                    code: "node_layout_unsupported".to_string(),
                    message: error.to_string(),
                },
                EnvironmentRecoveryActionReport {
                    code: "choose_supported_node_modules_layout".to_string(),
                    description:
                        "Use a single-package node_modules project with a matching supported lockfile"
                            .to_string(),
                    command: None,
                },
            )));
        }
        match select_node_source_lock(db, source_root, &root, &package) {
            Ok(Some(lock)) => {
                if let Err(error) = validate_source_node_lock(db, &lock) {
                    return Ok(Some(WorkspaceEnvironmentAdapterProposal::blocked(
                        EnvironmentProposalReasonReport {
                            code: "node_lock_unsupported".to_string(),
                            message: error.to_string(),
                        },
                        EnvironmentRecoveryActionReport {
                            code: "regenerate_supported_lock".to_string(),
                            description: "Regenerate the lockfile with the selected supported package manager without changing dependency intent".to_string(),
                            command: None,
                        },
                    )));
                }
                return Ok(Some(WorkspaceEnvironmentAdapterProposal::ready()));
            }
            Ok(None) => {}
            Err(error) => {
                return Ok(Some(WorkspaceEnvironmentAdapterProposal::blocked(
                    EnvironmentProposalReasonReport {
                        code: "node_lock_ambiguous".to_string(),
                        message: error.to_string(),
                    },
                    EnvironmentRecoveryActionReport {
                        code: "select_one_package_manager".to_string(),
                        description:
                            "Declare one packageManager and retain only its authoritative lockfile"
                                .to_string(),
                        command: None,
                    },
                )));
            }
        }
        let spec = match node_resolution_spec(db, source_root, &root) {
            Ok(spec) => spec,
            Err(error) => {
                return Ok(Some(WorkspaceEnvironmentAdapterProposal::blocked(
                    EnvironmentProposalReasonReport {
                        code: "node_resolution_unsupported".to_string(),
                        message: error.to_string(),
                    },
                    EnvironmentRecoveryActionReport {
                        code: "choose_supported_package_manager".to_string(),
                        description:
                            "Set packageManager to npm, pnpm, yarn, or bun and resolve a matching lock snapshot"
                                .to_string(),
                        command: None,
                    },
                )));
            }
        };
        if node_resolution_snapshot(db, source_root, &root, &spec)?.is_some() {
            return Ok(Some(WorkspaceEnvironmentAdapterProposal::ready()));
        }
        Ok(Some(WorkspaceEnvironmentAdapterProposal::resolvable(
            EnvironmentProposalReasonReport {
                code: "resolution_snapshot_missing".to_string(),
                message: "package.json is present but no supported package-manager lockfile or Trail-managed resolution snapshot is available".to_string(),
            },
            EnvironmentRecoveryActionReport {
                code: "resolve_component".to_string(),
                description: format!("Resolve and pin a Trail-managed {} snapshot without adding `{}` to source", spec.manager, spec.lock_name),
                command: None,
            },
        )))
    }

    fn resolution_plan(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<Option<ArtifactResolutionPlanV1>> {
        let root = normalize_package_root(component_root)?;
        let package_path = join_repo_path(&root, "package.json");
        let Some(package) = db.root_file_entry(source_root, &package_path)? else {
            return Ok(None);
        };
        let spec = node_resolution_spec(db, source_root, &root)?;
        let manager_tool = resolve_workspace_tool_executable(spec.manager)?;
        let policy_identity = sha256_hex(
            format!(
                "node-lock-resolver-v1\0{}\0{}\0ignore-scripts",
                spec.manager, manager_tool.identity
            )
            .as_bytes(),
        );
        Ok(Some(ArtifactResolutionPlanV1 {
            version: ARTIFACT_RESOLUTION_PLAN_VERSION,
            proposal_key: node_resolution_proposal_key(source_root, &root, &spec),
            source_root: source_root.clone(),
            component_id: self.component_id(&root)?,
            adapter_identity: self.identity().to_string(),
            policy_identity,
            program: spec.manager.to_string(),
            resolved_program: manager_tool.path.to_string_lossy().into_owned(),
            executable_identity: manager_tool.identity,
            argv: node_lock_resolution_argv(&spec)
                .into_iter()
                .map(str::to_string)
                .collect(),
            working_directory: if root.is_empty() {
                ".".to_string()
            } else {
                root.clone()
            },
            readable_inputs: vec![ArtifactResolutionInputV1 {
                source_path: package_path,
                content_hash: package.content_hash,
                size_bytes: package.size_bytes,
            }],
            candidate_output: join_repo_path(&root, spec.lock_name),
            allowed_authorities: Vec::new(),
            credential_handles: Vec::new(),
            script_policy: ArtifactScriptPolicyV1::Deny,
            environment_roles: BTreeMap::new(),
            limits: ArtifactActionLimitsV1 {
                timeout_ms: 10 * 60 * 1_000,
                stdout_bytes: 1024 * 1024,
                stderr_bytes: 1024 * 1024,
                candidate_bytes: 64 * 1024 * 1024,
                candidate_entries: 1,
                child_processes: 64,
            },
            snapshot_format: spec.snapshot_format(),
            validations: vec![ArtifactValidationV1 {
                name: format!("{}-lock-structure", spec.manager),
                kind: ArtifactValidationKindV1::Framework,
                required: true,
                parameters: BTreeMap::from([
                    ("manager".to_string(), spec.manager.to_string()),
                    ("lockfile".to_string(), spec.lock_name.to_string()),
                ]),
            }],
        }))
    }

    fn plan(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<WorkspaceEnvironmentPlan> {
        db.node_environment_plan(source_root, component_root)
    }
}

impl Trail {
    /// Compatibility entry point retained for existing `trail deps sync`
    /// callers. All execution and persistence is owned by the generic host.
    pub fn sync_node_dependencies(
        &self,
        lane: &str,
        package_root: Option<&str>,
    ) -> Result<WorkspaceLayerReport> {
        self.sync_workspace_environment(lane, "trail/node@1", package_root)
    }

    fn node_environment_plan(
        &self,
        root_id: &ObjectId,
        package_root: &str,
    ) -> Result<WorkspaceEnvironmentPlan> {
        let package_root = if package_root.trim_matches('/').is_empty() {
            String::new()
        } else {
            normalize_relative_path(package_root)?
        };
        let package_json = join_repo_path(&package_root, "package.json");
        let package_entry = self
            .root_file_entry(root_id, &package_json)?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "Node package root `{}` has no package.json",
                    if package_root.is_empty() {
                        "."
                    } else {
                        &package_root
                    }
                ))
            })?;
        let package_projection = self.project_entry_file(&package_entry)?;
        let package_text = fs::read_to_string(package_projection)?;
        let package_value: serde_json::Value = serde_json::from_str(&package_text)?;
        validate_node_modules_layout(self, root_id, &package_root, &package_value)?;
        let selected = select_node_source_lock(self, root_id, &package_root, &package_value)?;
        let selected_for_lifecycle = selected.clone();
        let component_id = NODE_WORKSPACE_ADAPTER.component_id(&package_root)?;
        let (
            lock_path,
            manager,
            lock_content_hash,
            lock_authority,
            source_lock_entry,
            resolution_inputs,
            source_projection,
        ) = if let Some(lock) = selected {
            validate_source_node_lock(self, &lock)?;
            (
                lock.path,
                lock.manager.to_string(),
                lock.entry.content_hash.clone(),
                "source".to_string(),
                Some(lock.entry),
                Vec::new(),
                None,
            )
        } else {
            let spec = node_resolution_spec_from_package(&package_value)?;
            let resolution_plan = NODE_WORKSPACE_ADAPTER
                .resolution_plan(self, root_id, &package_root)?
                .ok_or_else(|| {
                    Error::Corrupt(format!(
                        "Node component `{}` lost its resolver plan",
                        display_package_root(&package_root)
                    ))
                })?;
            let (snapshot_id, snapshot, bytes) = node_resolution_snapshot(
                    self,
                    root_id,
                    &package_root,
                    &spec,
                )?
                .ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "Node component `{}` has no supported lockfile or Trail-managed resolution snapshot; resolve it through Trail's artifact resolution operation before synchronizing",
                        display_package_root(&package_root)
                    ))
                })?;
            if snapshot.resolver_executable_identity != resolution_plan.executable_identity
                || snapshot.policy_identity != resolution_plan.policy_identity
            {
                return Err(Error::InvalidInput(format!(
                        "Node component `{}` resolution snapshot was created by a different package-manager executable or resolver policy; resolve it again for the current tool",
                        display_package_root(&package_root)
                    )));
            }
            let size_bytes = u64::try_from(bytes.len()).map_err(|_| {
                Error::InvalidInput("Node lock snapshot exceeds platform limits".to_string())
            })?;
            let lock_path = join_repo_path(&package_root, spec.lock_name);
            (
                lock_path.clone(),
                spec.manager.to_string(),
                snapshot.content_sha256.clone(),
                format!("snapshot:{}", snapshot_id.0),
                None,
                vec![WorkspaceEnvironmentResolutionInput {
                    snapshot_id,
                    source_root: root_id.clone(),
                    source_path: lock_path.clone(),
                    staging_path: format!("project/{lock_path}"),
                    content_hash: snapshot.content_sha256,
                    size_bytes,
                }],
                Some((root_id.clone(), "project".to_string())),
            )
        };
        let manager_version = tool_version(&manager)?;
        let node_version = tool_version("node")?;
        let node_tool = resolve_workspace_tool_executable("node")?;
        let manager_tool = resolve_workspace_tool_executable(&manager)?;
        validate_declared_package_manager_version(&package_value, &manager, &manager_version)?;
        let lifecycle_policy_path = join_repo_path(&package_root, NODE_LIFECYCLE_POLICY_FILE);
        let lifecycle_policy_present = self
            .root_file_entry(root_id, &lifecycle_policy_path)?
            .is_some();
        let approved_lifecycle = match selected_for_lifecycle.as_ref() {
            Some(lock) => approved_node_lifecycle(
                self,
                root_id,
                &package_root,
                &package_value,
                &manager,
                &manager_version,
                lock,
            )?,
            None if lifecycle_policy_present => {
                return Err(Error::InvalidInput(format!(
                    "Node lifecycle approval `{lifecycle_policy_path}` requires a committed source lockfile"
                )));
            }
            None => None,
        };
        if manager == "pnpm"
            && self
                .root_file_entry(
                    root_id,
                    &join_repo_path(&package_root, "pnpm-workspace.yaml"),
                )?
                .is_some()
        {
            return Err(Error::InvalidInput(format!(
                "Node component `{}` is a pnpm workspace root; synchronize a supported leaf package with its own lockfile until the monorepo adapter is enabled",
                display_package_root(&package_root)
            )));
        }
        if manager == "yarn" && !manager_version.starts_with("1.") {
            return Err(Error::InvalidInput(
                "Yarn Berry is not a supported node_modules contract; Trail's built-in adapter currently requires Yarn Classic 1.x"
                    .to_string(),
            ));
        }
        let mut files = BTreeMap::from([(package_json.clone(), package_entry.clone())]);
        if let Some(lock_entry) = source_lock_entry {
            files.insert(lock_path.clone(), lock_entry);
        }
        for name in [
            ".npmrc",
            ".yarnrc",
            "pnpmfile.cjs",
            "bunfig.toml",
            ".node-version",
            ".nvmrc",
        ] {
            let path = join_repo_path(&package_root, name);
            if let Some(entry) = self.root_file_entry(root_id, &path)? {
                files.insert(path, entry);
            }
        }
        let implementation_version = env!("CARGO_PKG_VERSION").to_string();
        let distribution_digest = "builtin:node-plan-v3".to_string();
        if let Some(approval) = &approved_lifecycle {
            files.insert(approval.path.clone(), approval.entry.clone());
        }
        let mut key_inputs = files
            .iter()
            .map(|(path, entry)| (path.clone(), entry.content_hash.clone()))
            .collect::<BTreeMap<_, _>>();
        key_inputs.insert(lock_path.clone(), lock_content_hash);
        key_inputs.insert("lock_authority".to_string(), lock_authority);
        key_inputs.insert(
            "adapter_implementation".to_string(),
            implementation_version.clone(),
        );
        key_inputs.insert(
            "adapter_distribution_digest".to_string(),
            distribution_digest.clone(),
        );
        key_inputs.insert(
            "command_environment".to_string(),
            "npm_config_cache=cache:package-manager/npm;PNPM_HOME=cache:package-manager/pnpm-home;PNPM_STORE_DIR=cache:package-manager/pnpm-store;YARN_CACHE_FOLDER=cache:package-manager/yarn;BUN_INSTALL_CACHE_DIR=cache:package-manager/bun;TRAIL_NODE=tool:node;TRAIL_NPM=tool?:npm;TRAIL_PNPM=tool?:pnpm;TRAIL_YARN=tool?:yarn;TRAIL_BUN=tool?:bun;TRAIL_NODE_MODULES=direct:node_modules;NODE_PATH=direct:node_modules;PATH+=direct:node_modules/.bin+tool-dirs".to_string(),
        );
        let mut lifecycle_tools = BTreeMap::new();
        let mut lifecycle_environment = BTreeMap::new();
        if let Some(approval) = &approved_lifecycle {
            let script_text = package_value
                .get("scripts")
                .and_then(serde_json::Value::as_object)
                .and_then(|scripts| scripts.get(&approval.script))
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| Error::Corrupt("approved Node script disappeared".to_string()))?;
            if !approved_node_native_script(script_text) {
                return Err(Error::InvalidInput(format!(
                    "approved Node lifecycle script `{}` must be a single literal `node-gyp rebuild` command without shell operators",
                    approval.script
                )));
            }
            let node_abi = node_expression("process.versions.modules")?;
            let node_installation = fs::canonicalize(&node_tool.path)?
                .parent()
                .and_then(Path::parent)
                .map(Path::to_path_buf)
                .ok_or_else(|| {
                    Error::InvalidInput("Node executable has no installation root".to_string())
                })?;
            let node_headers = node_installation.join("include/node");
            let node_headers_identity = node_header_tree_identity(&node_headers)?;
            let compiler = resolve_workspace_tool_executable("cc")?;
            let compiler_version = tool_version_path(&compiler.path)?;
            let python = resolve_workspace_tool_executable("python3")?;
            let make = resolve_workspace_tool_executable("make")?;
            key_inputs.insert(
                "lifecycle:approval_sha256".to_string(),
                approval.digest.clone(),
            );
            key_inputs.insert("lifecycle:approval_path".to_string(), approval.path.clone());
            key_inputs.insert("lifecycle:source_root".to_string(), root_id.0.clone());
            key_inputs.insert("lifecycle:script".to_string(), approval.script.clone());
            key_inputs.insert("lifecycle:outputs".to_string(), approval.outputs.join("\0"));
            key_inputs.insert("lifecycle:network".to_string(), "deny".to_string());
            lifecycle_tools.extend([
                ("node-abi".to_string(), node_abi),
                ("node-headers".to_string(), node_headers_identity),
                ("native-compiler".to_string(), compiler_version),
                ("native-compiler-executable".to_string(), compiler.identity),
                ("native-make-executable".to_string(), make.identity),
                ("native-python-executable".to_string(), python.identity),
            ]);
            lifecycle_environment.insert("npm_config_offline".to_string(), "true".to_string());
            lifecycle_environment.insert("npm_config_audit".to_string(), "false".to_string());
            lifecycle_environment.insert("npm_config_fund".to_string(), "false".to_string());
            lifecycle_environment.insert(
                "npm_config_nodedir".to_string(),
                node_installation.to_string_lossy().into_owned(),
            );
        }
        let mut tool_versions = BTreeMap::from([
            ("node".to_string(), node_version),
            (manager.clone(), manager_version),
            ("node-executable".to_string(), node_tool.identity),
            (
                format!("{manager}-executable"),
                manager_tool.identity.clone(),
            ),
        ]);
        tool_versions.extend(lifecycle_tools);
        let key = WorkspaceLayerKeyV1 {
            kind: "dependency".to_string(),
            adapter: "node".to_string(),
            adapter_version: 1,
            inputs: key_inputs,
            tool_versions,
            platform: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            portability_scope: "platform-architecture-node-abi".to_string(),
            strategy: if approved_lifecycle.is_some() {
                format!("{manager}-frozen-approved-native-lifecycle-v1")
            } else {
                format!("{manager}-frozen-ignore-scripts-v1")
            },
        };
        let project = "project".to_string();
        let cache = self.declare_workspace_environment_cache(
            NODE_WORKSPACE_ADAPTER.identity(),
            "package-manager",
            WorkspaceEnvironmentCacheProtocol::ContentStore,
            WorkspaceEnvironmentCacheAccess::ToolConcurrent,
            BTreeMap::from([
                ("manager".to_string(), manager.clone()),
                (
                    "manager_executable".to_string(),
                    manager_tool.identity.clone(),
                ),
                ("platform".to_string(), std::env::consts::OS.to_string()),
                (
                    "architecture".to_string(),
                    std::env::consts::ARCH.to_string(),
                ),
            ]),
        )?;
        let cache_root = &cache.storage_path;
        let mut environment = BTreeMap::from([
            (
                "npm_config_cache".to_string(),
                cache_root.join("npm").to_string_lossy().into_owned(),
            ),
            (
                "PNPM_HOME".to_string(),
                cache_root.join("pnpm-home").to_string_lossy().into_owned(),
            ),
            (
                "PNPM_STORE_DIR".to_string(),
                cache_root.join("pnpm-store").to_string_lossy().into_owned(),
            ),
            (
                "YARN_CACHE_FOLDER".to_string(),
                cache_root.join("yarn").to_string_lossy().into_owned(),
            ),
            (
                "BUN_INSTALL_CACHE_DIR".to_string(),
                cache_root.join("bun").to_string_lossy().into_owned(),
            ),
        ]);
        environment.extend(lifecycle_environment);
        let args = match manager.as_str() {
            "npm" => vec!["ci", "--ignore-scripts", "--no-audit", "--no-fund"],
            "pnpm" => vec![
                "install",
                "--ignore-workspace",
                "--frozen-lockfile",
                "--ignore-scripts",
            ],
            "yarn" => vec![
                "install",
                "--frozen-lockfile",
                "--ignore-scripts",
                "--non-interactive",
            ],
            "bun" => vec![
                "install",
                "--frozen-lockfile",
                "--ignore-scripts",
                "--no-progress",
            ],
            other => {
                return Err(Error::InvalidInput(format!(
                    "unsupported Node package manager `{other}`"
                )));
            }
        }
        .into_iter()
        .map(str::to_string)
        .collect();
        let lifecycle_enabled = approved_lifecycle.is_some();
        let source_projection = if lifecycle_enabled {
            let mut file_count = 0usize;
            let mut source_bytes = 0u64;
            self.visit_root_file_entries(root_id, &[], |path, entry| {
                file_count = file_count.checked_add(1).ok_or_else(|| {
                    Error::InvalidInput("Node lifecycle source count overflowed".to_string())
                })?;
                source_bytes = source_bytes.checked_add(entry.size_bytes).ok_or_else(|| {
                    Error::InvalidInput("Node lifecycle source size overflowed".to_string())
                })?;
                if file_count > MAX_NODE_LIFECYCLE_SOURCE_FILES
                    || source_bytes > MAX_NODE_LIFECYCLE_SOURCE_BYTES
                {
                    return Err(Error::InvalidInput(format!(
                        "approved Node lifecycle source exceeds {} files or {} bytes",
                        MAX_NODE_LIFECYCLE_SOURCE_FILES, MAX_NODE_LIFECYCLE_SOURCE_BYTES
                    )));
                }
                if path == ".trail" || path.starts_with(".trail/") {
                    return Err(Error::InvalidPath {
                        path,
                        reason: "Trail private state cannot enter a lifecycle source closure"
                            .to_string(),
                    });
                }
                Ok(())
            })?;
            Some((root_id.clone(), "project".to_string()))
        } else {
            source_projection
        };
        let inputs = if source_projection.is_some() {
            Vec::new()
        } else {
            files
                .into_iter()
                .map(|(source_path, entry)| {
                    let relative = strip_package_root(&source_path, &package_root)?;
                    Ok(WorkspaceEnvironmentInput {
                        source_path,
                        staging_path: format!("project/{relative}"),
                        entry,
                    })
                })
                .collect::<Result<Vec<_>>>()?
        };
        let mount_path = if package_root.is_empty() {
            "node_modules".to_string()
        } else {
            format!("{package_root}/node_modules")
        };
        let project_root = if package_root.is_empty() {
            project.clone()
        } else {
            format!("{project}/{package_root}")
        };
        let mut outputs = vec![WorkspaceEnvironmentOutput {
            name: "modules".to_string(),
            output_path: format!("{project_root}/node_modules"),
            mount_path,
            policy: if lifecycle_enabled {
                WorkspaceEnvironmentOutputPolicy::WritablePrivate
            } else {
                WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate
            },
            reuse: if lifecycle_enabled {
                EnvironmentReuseMode::None
            } else {
                EnvironmentReuseMode::Exact
            },
            scope: if lifecycle_enabled {
                EnvironmentSharingScope::Lane
            } else {
                EnvironmentSharingScope::Workspace
            },
            publish: if lifecycle_enabled {
                EnvironmentPublicationTrigger::Never
            } else {
                EnvironmentPublicationTrigger::OnSync
            },
            gate: None,
            create_if_missing: true,
        }];
        if let Some(approval) = &approved_lifecycle {
            for (index, relative) in approval.outputs.iter().enumerate() {
                outputs.push(WorkspaceEnvironmentOutput {
                    name: format!("lifecycle-output-{index}"),
                    output_path: format!("{project_root}/{relative}"),
                    mount_path: join_repo_path(&package_root, relative),
                    policy: WorkspaceEnvironmentOutputPolicy::WritablePrivate,
                    reuse: EnvironmentReuseMode::None,
                    scope: EnvironmentSharingScope::Lane,
                    publish: EnvironmentPublicationTrigger::Never,
                    gate: None,
                    create_if_missing: true,
                });
            }
        }
        let install_command = WorkspaceEnvironmentCommand {
            program: manager.clone(),
            resolved_program: manager_tool.path.clone(),
            executable_identity: manager_tool.identity.clone(),
            args,
            working_directory: project_root.clone(),
            environment: environment.clone(),
            remove_environment: Vec::new(),
            cache_names: vec![cache.name.clone()],
        };
        let lifecycle_command =
            approved_lifecycle
                .as_ref()
                .map(|approval| WorkspaceEnvironmentCommand {
                    program: manager.clone(),
                    resolved_program: manager_tool.path.clone(),
                    executable_identity: manager_tool.identity.clone(),
                    args: vec![
                        "run-script".to_string(),
                        approval.script.clone(),
                        "--ignore-scripts".to_string(),
                        "--offline".to_string(),
                    ],
                    working_directory: project_root.clone(),
                    environment: environment.clone(),
                    remove_environment: Vec::new(),
                    cache_names: vec![cache.name.clone()],
                });
        Ok(WorkspaceEnvironmentPlan {
            component_id,
            adapter_identity: NODE_WORKSPACE_ADAPTER.identity().to_string(),
            adapter_version: 1,
            implementation_version,
            distribution_digest,
            kind: "dependency".to_string(),
            dependencies: Vec::new(),
            resolved_dependencies: Vec::new(),
            layer_key: key,
            inputs,
            resolution_inputs,
            construction_seed: None,
            source_projection,
            pre_commands: if lifecycle_enabled {
                vec![install_command.clone()]
            } else {
                Vec::new()
            },
            command: if lifecycle_enabled {
                lifecycle_command
            } else {
                Some(install_command)
            },
            mounted_commands: Vec::new(),
            caches: vec![cache],
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: if lifecycle_enabled {
                WorkspaceEnvironmentSandboxPolicy::ApprovedLifecycle
            } else {
                WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin
            },
            outputs,
            stale_reason:
                "package, lockfile, Node runtime, package manager, or adapter policy changed"
                    .to_string(),
        })
    }
}

fn node_component_is_descendant_of_locked_workspace(
    db: &Trail,
    source_root: &ObjectId,
    component_root: &str,
) -> Result<bool> {
    if component_root.is_empty() {
        return Ok(false);
    }
    for (lock_name, _) in supported_lockfiles() {
        if db
            .root_file_entry(source_root, &join_repo_path(component_root, lock_name))?
            .is_some()
        {
            return Ok(false);
        }
    }

    let segments = component_root.split('/').collect::<Vec<_>>();
    for depth in (0..segments.len()).rev() {
        let ancestor = segments[..depth].join("/");
        let has_pnpm_workspace = db
            .root_file_entry(
                source_root,
                &join_repo_path(&ancestor, "pnpm-workspace.yaml"),
            )?
            .is_some()
            && db
                .root_file_entry(source_root, &join_repo_path(&ancestor, "pnpm-lock.yaml"))?
                .is_some();
        if has_pnpm_workspace {
            return Ok(true);
        }

        let package_path = join_repo_path(&ancestor, "package.json");
        let Some(package_entry) = db.root_file_entry(source_root, &package_path)? else {
            continue;
        };
        let package_bytes = db.materialize_entry_bytes(&package_entry)?;
        let package: serde_json::Value =
            serde_json::from_slice(&package_bytes).map_err(|error| {
                Error::InvalidInput(format!(
                    "Node manifest `{package_path}` is malformed JSON: {error}"
                ))
            })?;
        let declares_workspaces =
            package
                .get("workspaces")
                .is_some_and(|workspaces| match workspaces {
                    serde_json::Value::Array(values) => !values.is_empty(),
                    serde_json::Value::Object(values) => values
                        .get("packages")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|packages| !packages.is_empty()),
                    _ => false,
                });
        if declares_workspaces {
            for (lock_name, _) in supported_lockfiles() {
                if db
                    .root_file_entry(source_root, &join_repo_path(&ancestor, lock_name))?
                    .is_some()
                {
                    return Ok(true);
                }
            }
        }
    }
    Ok(false)
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DeclaredNodeManager {
    name: String,
    version: Option<String>,
}

#[derive(Clone, Debug)]
struct NodeSourceLock {
    path: String,
    manager: &'static str,
    entry: FileEntry,
}

fn approved_node_lifecycle(
    db: &Trail,
    source_root: &ObjectId,
    package_root: &str,
    package: &serde_json::Value,
    manager: &str,
    manager_version: &str,
    lock: &NodeSourceLock,
) -> Result<Option<ApprovedNodeLifecycle>> {
    let path = join_repo_path(package_root, NODE_LIFECYCLE_POLICY_FILE);
    let Some(entry) = db.root_file_entry(source_root, &path)? else {
        return Ok(None);
    };
    if entry.size_bytes > 64 * 1024 {
        return Err(Error::InvalidInput(format!(
            "Node lifecycle approval `{path}` exceeds 64 KiB"
        )));
    }
    let bytes = db.materialize_entry_bytes(&entry)?;
    let policy: NodeLifecyclePolicyV1 = serde_json::from_slice(&bytes).map_err(|error| {
        Error::InvalidInput(format!(
            "Node lifecycle approval `{path}` is malformed: {error}"
        ))
    })?;
    if policy.version != 1 {
        return Err(Error::InvalidInput(format!(
            "Node lifecycle approval `{path}` uses unsupported version {}; expected 1",
            policy.version
        )));
    }
    if manager != "npm" {
        return Err(Error::InvalidInput(format!(
            "Node lifecycle approval `{path}` currently supports only exact npm installs; `{manager}` remains script-disabled"
        )));
    }
    let expected_manager = format!("{manager}@{}", manager_version.trim_start_matches('v'));
    if policy.manager != expected_manager {
        return Err(Error::InvalidInput(format!(
            "Node lifecycle approval `{path}` selects manager `{}` but Trail resolved `{expected_manager}`",
            policy.manager
        )));
    }
    let lock_bytes = db.materialize_entry_bytes(&lock.entry)?;
    let lock_sha256 = sha256_hex(&lock_bytes);
    if policy.lock_sha256 != lock_sha256
        || policy.lock_sha256.len() != 64
        || !policy
            .lock_sha256
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        return Err(Error::InvalidInput(format!(
            "Node lifecycle approval `{path}` does not match the exact SHA-256 of `{}`",
            lock.path
        )));
    }
    if policy.capabilities.network != "deny" || !policy.capabilities.native_toolchain {
        return Err(Error::InvalidInput(format!(
            "Node lifecycle approval `{path}` must declare capabilities.network = \"deny\" and native_toolchain = true"
        )));
    }
    let lock_document: serde_json::Value =
        serde_json::from_slice(&lock_bytes).map_err(|error| {
            Error::InvalidInput(format!(
                "approved Node lifecycle lock `{}` is malformed: {error}",
                lock.path
            ))
        })?;
    let node_gyp = lock_document
        .pointer("/packages/node_modules~1node-gyp")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "Node lifecycle approval `{path}` requires lock-pinned `node-gyp`"
            ))
        })?;
    let node_gyp_version = node_gyp
        .get("version")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Error::InvalidInput("locked node-gyp has no version".to_string()))?;
    let node_gyp_integrity = node_gyp
        .get("integrity")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| Error::InvalidInput("locked node-gyp has no integrity".to_string()))?;
    let expected_package = format!("node-gyp@{node_gyp_version}#{node_gyp_integrity}");
    if policy.packages != [expected_package.clone()] {
        return Err(Error::InvalidInput(format!(
            "Node lifecycle approval `{path}` must select exactly locked package `{expected_package}`"
        )));
    }
    if policy.scripts.len() != 1
        || policy.scripts[0].package != "."
        || !matches!(
            policy.scripts[0].phase.as_str(),
            "preinstall" | "install" | "postinstall"
        )
    {
        return Err(Error::InvalidInput(format!(
            "Node lifecycle approval `{path}` must select exactly one root preinstall, install, or postinstall script"
        )));
    }
    let script = policy.scripts[0].phase.clone();
    let declared_script = package
        .get("scripts")
        .and_then(serde_json::Value::as_object)
        .and_then(|scripts| scripts.get(&script))
        .and_then(serde_json::Value::as_str)
        .filter(|value| !value.trim().is_empty());
    if declared_script.is_none() {
        return Err(Error::InvalidInput(format!(
            "Node lifecycle approval `{path}` selects missing or empty root script `{script}`"
        )));
    }
    if policy.outputs.is_empty() || policy.outputs.len() > 8 {
        return Err(Error::InvalidInput(format!(
            "Node lifecycle approval `{path}` must declare between 1 and 8 private output directories"
        )));
    }
    let mut outputs = BTreeSet::new();
    for output in policy.outputs {
        let output = normalize_relative_path(&output)?;
        if output == "node_modules"
            || output == "package.json"
            || output == NODE_LIFECYCLE_POLICY_FILE
            || output == lock.path
            || output.starts_with("node_modules/")
        {
            return Err(Error::InvalidInput(format!(
                "Node lifecycle approval `{path}` contains reserved output `{output}`"
            )));
        }
        let source_path = join_repo_path(package_root, &output);
        let mut tracked = false;
        db.visit_root_file_entries(source_root, std::slice::from_ref(&source_path), |_, _| {
            tracked = true;
            Ok(())
        })?;
        if tracked {
            return Err(Error::InvalidInput(format!(
                "Node lifecycle output `{source_path}` overlaps committed source"
            )));
        }
        if !outputs.insert(output.clone()) {
            return Err(Error::InvalidInput(format!(
                "Node lifecycle approval `{path}` repeats output `{output}`"
            )));
        }
    }
    Ok(Some(ApprovedNodeLifecycle {
        path,
        entry,
        digest: sha256_hex(&bytes),
        script,
        outputs: outputs.into_iter().collect(),
    }))
}

fn declared_node_manager(package: &serde_json::Value) -> Result<Option<DeclaredNodeManager>> {
    let Some(identity) = package
        .get("packageManager")
        .and_then(serde_json::Value::as_str)
    else {
        return Ok(None);
    };
    let (name, raw_version) = identity
        .split_once('@')
        .map_or((identity, None), |(name, version)| (name, Some(version)));
    if !matches!(name, "npm" | "pnpm" | "yarn" | "bun") {
        return Err(Error::InvalidInput(format!(
            "Node packageManager `{identity}` is unsupported; expected npm, pnpm, yarn, or bun"
        )));
    }
    let version = raw_version
        .map(|version| version.split_once('+').map_or(version, |(base, _)| base))
        .filter(|version| !version.is_empty())
        .map(str::to_string);
    if identity.contains('@') && version.is_none() {
        return Err(Error::InvalidInput(format!(
            "Node packageManager `{identity}` has an empty version"
        )));
    }
    Ok(Some(DeclaredNodeManager {
        name: name.to_string(),
        version,
    }))
}

fn select_node_source_lock(
    db: &Trail,
    source_root: &ObjectId,
    package_root: &str,
    package: &serde_json::Value,
) -> Result<Option<NodeSourceLock>> {
    let declared = declared_node_manager(package)?;
    let mut locks = Vec::new();
    for (name, manager) in supported_lockfiles() {
        let path = join_repo_path(package_root, name);
        if let Some(entry) = db.root_file_entry(source_root, &path)? {
            locks.push(NodeSourceLock {
                path,
                manager,
                entry,
            });
        }
    }
    if locks.is_empty() {
        return Ok(None);
    }

    if let Some(declared) = declared.as_ref() {
        let mismatched = locks
            .iter()
            .filter(|lock| lock.manager != declared.name)
            .map(|lock| lock.path.as_str())
            .collect::<Vec<_>>();
        let matching = locks
            .iter()
            .filter(|lock| lock.manager == declared.name)
            .collect::<Vec<_>>();
        if matching.is_empty() {
            return Err(Error::InvalidInput(format!(
                "packageManager selects `{}` but the repository lockfile(s) belong to another manager: {}",
                declared.name,
                mismatched.join(", ")
            )));
        }
        locks.retain(|lock| lock.manager == declared.name);
    } else {
        let managers = locks
            .iter()
            .map(|lock| lock.manager)
            .collect::<BTreeSet<_>>();
        if managers.len() > 1 {
            return Err(Error::InvalidInput(format!(
                "Node component `{}` contains lockfiles for multiple package managers ({}); declare packageManager and retain one authoritative manager lock",
                display_package_root(package_root),
                managers.into_iter().collect::<Vec<_>>().join(", ")
            )));
        }
    }

    if locks.len() > 1 && locks[0].manager != "npm" {
        return Err(Error::InvalidInput(format!(
            "Node component `{}` contains multiple `{}` lock authorities ({}); retain exactly one",
            display_package_root(package_root),
            locks[0].manager,
            locks
                .iter()
                .map(|lock| lock.path.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )));
    }
    // npm-shrinkwrap.json intentionally precedes package-lock.json and is npm's
    // published lock authority when both are present.
    Ok(locks.into_iter().next())
}

fn validate_declared_package_manager_version(
    package: &serde_json::Value,
    selected_manager: &str,
    installed_version: &str,
) -> Result<()> {
    let Some(declared) = declared_node_manager(package)? else {
        return Ok(());
    };
    if declared.name != selected_manager {
        return Err(Error::InvalidInput(format!(
            "packageManager selects `{}` but Trail selected `{selected_manager}`",
            declared.name
        )));
    }
    if let Some(version) = declared.version
        && version != installed_version
    {
        return Err(Error::InvalidInput(format!(
            "packageManager requires `{selected_manager}@{version}` but the resolved executable reports `{installed_version}`; activate the exact manager version before synchronizing"
        )));
    }
    Ok(())
}

fn validate_node_modules_layout(
    db: &Trail,
    source_root: &ObjectId,
    package_root: &str,
    package: &serde_json::Value,
) -> Result<()> {
    if package.get("workspaces").is_some() {
        return Err(Error::InvalidInput(format!(
            "Node component `{}` declares workspaces; synchronize a supported leaf package explicitly until the monorepo adapter is enabled",
            display_package_root(package_root)
        )));
    }
    if contains_local_node_dependency(package) {
        return Err(Error::InvalidInput(format!(
            "Node component `{}` contains file:, link:, or workspace: dependencies that cannot be represented by an isolated node_modules layer",
            display_package_root(package_root)
        )));
    }

    let yarn_requested = declared_node_manager(package)?
        .is_some_and(|manager| manager.name == "yarn")
        || db
            .root_file_entry(source_root, &join_repo_path(package_root, "yarn.lock"))?
            .is_some();
    if !yarn_requested {
        return Ok(());
    }
    for marker in [".yarnrc.yml", ".pnp.js", ".pnp.cjs"] {
        if db
            .root_file_entry(source_root, &join_repo_path(package_root, marker))?
            .is_some()
        {
            return Err(Error::InvalidInput(format!(
                "Yarn Berry/PnP marker `{marker}` is unsupported by Trail's node_modules adapter; use Yarn Classic without PnP"
            )));
        }
    }
    if package
        .get("installConfig")
        .and_then(serde_json::Value::as_object)
        .and_then(|config| config.get("pnp"))
        .and_then(serde_json::Value::as_bool)
        == Some(true)
    {
        return Err(Error::InvalidInput(
            "Yarn Classic Plug'n'Play selected by package.json installConfig.pnp is unsupported by Trail's node_modules adapter"
                .to_string(),
        ));
    }
    let yarnrc_path = join_repo_path(package_root, ".yarnrc");
    if let Some(entry) = db.root_file_entry(source_root, &yarnrc_path)? {
        let text = String::from_utf8(db.materialize_entry_bytes(&entry)?).map_err(|_| {
            Error::InvalidInput(format!("Yarn configuration `{yarnrc_path}` is not UTF-8"))
        })?;
        if text.lines().any(|line| {
            let normalized = line.trim().to_ascii_lowercase();
            normalized.contains("pnp")
                && !normalized.ends_with(" false")
                && !normalized.ends_with("=false")
        }) {
            return Err(Error::InvalidInput(format!(
                "Yarn configuration `{yarnrc_path}` selects or ambiguously configures Plug'n'Play; Trail requires an explicit node_modules layout"
            )));
        }
    }
    Ok(())
}

fn validate_source_node_lock(db: &Trail, lock: &NodeSourceLock) -> Result<()> {
    let bytes = db.materialize_entry_bytes(&lock.entry)?;
    if lock.path.ends_with("bun.lockb") {
        if bytes.is_empty() {
            return Err(Error::InvalidInput(
                "Bun binary lockfile bun.lockb is empty".to_string(),
            ));
        }
        return Ok(());
    }
    let lock_name = supported_lockfiles()
        .into_iter()
        .map(|(name, _)| name)
        .find(|name| lock.path.ends_with(name))
        .ok_or_else(|| Error::Corrupt(format!("unknown Node lock path `{}`", lock.path)))?;
    validate_node_lock_snapshot(
        &NodeResolutionSpec {
            manager: lock.manager,
            lock_name,
        },
        &bytes,
    )
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct NodeResolutionSpec {
    manager: &'static str,
    lock_name: &'static str,
}

impl NodeResolutionSpec {
    fn snapshot_format(self) -> String {
        format!("node-lock-{}-{}-v1", self.manager, self.lock_name)
    }
}

fn node_resolution_spec(
    db: &Trail,
    source_root: &ObjectId,
    package_root: &str,
) -> Result<NodeResolutionSpec> {
    let package_path = join_repo_path(package_root, "package.json");
    let package = db
        .root_file_entry(source_root, &package_path)?
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "Node component `{}` has no package.json",
                display_package_root(package_root)
            ))
        })?;
    let projection = db.project_entry_file(&package)?;
    let document: serde_json::Value = serde_json::from_slice(&fs::read(projection)?)?;
    node_resolution_spec_from_package(&document)
}

fn node_resolution_spec_from_package(package: &serde_json::Value) -> Result<NodeResolutionSpec> {
    let declared = declared_node_manager(package)?;
    let manager = declared
        .as_ref()
        .map_or("npm", |manager| manager.name.as_str());
    match manager {
        "npm" => Ok(NodeResolutionSpec {
            manager: "npm",
            lock_name: "package-lock.json",
        }),
        "pnpm" => Ok(NodeResolutionSpec {
            manager: "pnpm",
            lock_name: "pnpm-lock.yaml",
        }),
        "yarn" => Ok(NodeResolutionSpec {
            manager: "yarn",
            lock_name: "yarn.lock",
        }),
        "bun" => Ok(NodeResolutionSpec {
            manager: "bun",
            lock_name: "bun.lock",
        }),
        other => Err(Error::InvalidInput(format!(
            "Node packageManager `{other}` is unsupported; expected npm, pnpm, yarn, or bun"
        ))),
    }
}

fn node_lock_resolution_argv(spec: &NodeResolutionSpec) -> Vec<&'static str> {
    match spec.manager {
        "npm" => vec![
            "npm",
            "install",
            "--package-lock-only",
            "--ignore-scripts",
            "--no-audit",
            "--no-fund",
        ],
        "pnpm" => vec!["pnpm", "install", "--lockfile-only", "--ignore-scripts"],
        "yarn" => vec!["yarn", "install", "--ignore-scripts"],
        "bun" => vec!["bun", "install", "--lockfile-only", "--ignore-scripts"],
        _ => Vec::new(),
    }
}

fn node_resolution_proposal_key(
    source_root: &ObjectId,
    package_root: &str,
    spec: &NodeResolutionSpec,
) -> String {
    let identity = format!(
        "node-lock-resolution-v1\0{}\0{}\0{}\0{}\0{}",
        source_root.0,
        package_root,
        NODE_WORKSPACE_ADAPTER_METADATA.canonical_identity,
        spec.manager,
        spec.lock_name
    );
    format!("node_lock_v1_{}", sha256_hex(identity.as_bytes()))
}

fn node_resolution_snapshot(
    db: &Trail,
    source_root: &ObjectId,
    package_root: &str,
    spec: &NodeResolutionSpec,
) -> Result<Option<(ObjectId, ArtifactResolutionSnapshotV1, Vec<u8>)>> {
    let proposal_key = node_resolution_proposal_key(source_root, package_root, spec);
    let expected_component = NODE_WORKSPACE_ADAPTER.component_id(package_root)?;
    let snapshot_format = spec.snapshot_format();
    db.verified_workspace_environment_resolution_snapshot(
        &proposal_key,
        source_root,
        &expected_component,
        NODE_WORKSPACE_ADAPTER.identity(),
        &snapshot_format,
        |bytes| validate_node_lock_snapshot(spec, bytes),
    )
}

fn validate_node_lock_snapshot(spec: &NodeResolutionSpec, bytes: &[u8]) -> Result<()> {
    if bytes.is_empty() {
        return Err(Error::InvalidInput(format!(
            "Trail-managed {} lock snapshot is empty",
            spec.manager
        )));
    }
    match spec.manager {
        "npm" => {
            let document: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| {
                Error::InvalidInput(format!(
                    "Trail-managed npm lock snapshot is malformed JSON: {error}"
                ))
            })?;
            let version = document
                .get("lockfileVersion")
                .and_then(serde_json::Value::as_u64)
                .ok_or_else(|| {
                    Error::InvalidInput(
                        "Trail-managed npm lock snapshot has no lockfileVersion".to_string(),
                    )
                })?;
            if !(1..=3).contains(&version) {
                return Err(Error::InvalidInput(format!(
                    "Trail-managed npm lockfileVersion {version} is unsupported"
                )));
            }
        }
        "pnpm" => {
            let text = std::str::from_utf8(bytes).map_err(|_| {
                Error::InvalidInput(
                    "Trail-managed pnpm lock snapshot is not UTF-8 YAML".to_string(),
                )
            })?;
            if !text
                .lines()
                .any(|line| line.trim_start().starts_with("lockfileVersion:"))
            {
                return Err(Error::InvalidInput(
                    "Trail-managed pnpm lock snapshot has no lockfileVersion".to_string(),
                ));
            }
        }
        "yarn" => {
            let text = std::str::from_utf8(bytes).map_err(|_| {
                Error::InvalidInput("Trail-managed Yarn lock snapshot is not UTF-8".to_string())
            })?;
            if text.contains("__metadata:") || !text.contains("yarn lockfile v1") {
                return Err(Error::InvalidInput(
                    "Trail's built-in Node adapter requires a Yarn Classic v1 lockfile; Yarn Berry/PnP locks are unsupported"
                        .to_string(),
                ));
            }
        }
        "bun" => {
            let text = std::str::from_utf8(bytes).map_err(|_| {
                Error::InvalidInput("Trail-managed Bun text lock snapshot is not UTF-8".to_string())
            })?;
            if !text.contains("lockfileVersion") {
                return Err(Error::InvalidInput(
                    "Trail-managed Bun text lock snapshot has no lockfileVersion".to_string(),
                ));
            }
        }
        other => {
            return Err(Error::InvalidInput(format!(
                "Trail-managed Node lock snapshot uses unsupported manager `{other}`"
            )));
        }
    }
    Ok(())
}

fn supported_lockfiles() -> [(&'static str, &'static str); 6] {
    [
        ("pnpm-lock.yaml", "pnpm"),
        ("yarn.lock", "yarn"),
        ("bun.lock", "bun"),
        ("bun.lockb", "bun"),
        ("npm-shrinkwrap.json", "npm"),
        ("package-lock.json", "npm"),
    ]
}

fn normalize_package_root(package_root: &str) -> Result<String> {
    if package_root.trim_matches('/').is_empty() {
        Ok(String::new())
    } else {
        normalize_relative_path(package_root)
    }
}

fn display_package_root(package_root: &str) -> &str {
    if package_root.is_empty() {
        "."
    } else {
        package_root
    }
}

fn contains_local_node_dependency(package: &serde_json::Value) -> bool {
    [
        "dependencies",
        "devDependencies",
        "optionalDependencies",
        "peerDependencies",
    ]
    .into_iter()
    .filter_map(|name| package.get(name).and_then(serde_json::Value::as_object))
    .flat_map(|dependencies| dependencies.values())
    .filter_map(serde_json::Value::as_str)
    .any(|value| {
        ["file:", "link:", "workspace:"]
            .iter()
            .any(|prefix| value.starts_with(prefix))
    })
}

fn tool_version(tool: &str) -> Result<String> {
    let output = Command::new(tool)
        .arg("--version")
        .output()
        .map_err(|err| {
            Error::InvalidInput(format!("required tool `{tool}` is unavailable: {err}"))
        })?;
    if !output.status.success() {
        return Err(Error::InvalidInput(format!(
            "`{tool} --version` failed with {}",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn tool_version_path(tool: &Path) -> Result<String> {
    let output = Command::new(tool)
        .arg("--version")
        .output()
        .map_err(|err| {
            Error::InvalidInput(format!(
                "required tool `{}` is unavailable: {err}",
                tool.display()
            ))
        })?;
    if !output.status.success() {
        return Err(Error::InvalidInput(format!(
            "`{} --version` failed with {}",
            tool.display(),
            output.status
        )));
    }
    let text = if output.stdout.is_empty() {
        &output.stderr
    } else {
        &output.stdout
    };
    Ok(String::from_utf8_lossy(text).trim().to_string())
}

fn node_expression(expression: &str) -> Result<String> {
    let output = Command::new("node")
        .args(["-p", expression])
        .output()
        .map_err(|err| {
            Error::InvalidInput(format!("required tool `node` is unavailable: {err}"))
        })?;
    if !output.status.success() {
        return Err(Error::InvalidInput(format!(
            "`node -p {expression}` failed with {}",
            output.status
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn approved_node_native_script(script: &str) -> bool {
    let mut words = script.split_ascii_whitespace();
    if words.next() != Some("node-gyp") || words.next() != Some("rebuild") {
        return false;
    }
    words.all(|word| {
        !word.is_empty()
            && word.bytes().all(|byte| {
                byte.is_ascii_alphanumeric()
                    || matches!(byte, b'-' | b'_' | b'.' | b'/' | b'=' | b':')
            })
            && !word.starts_with('/')
            && !word.split('/').any(|segment| segment == "..")
    })
}

fn node_header_tree_identity(root: &Path) -> Result<String> {
    const MAX_FILES: usize = 4_096;
    const MAX_BYTES: u64 = 96 * 1024 * 1024;
    let root = fs::canonicalize(root).map_err(|error| {
        Error::InvalidInput(format!(
            "Node development headers are unavailable at `{}`: {error}",
            root.display()
        ))
    })?;
    if !root.join("node.h").is_file() {
        return Err(Error::InvalidInput(format!(
            "Node development headers at `{}` have no node.h",
            root.display()
        )));
    }
    let mut files = Vec::new();
    for entry in walkdir::WalkDir::new(&root).follow_links(false) {
        let entry = entry.map_err(|error| Error::InvalidInput(error.to_string()))?;
        let metadata = entry
            .metadata()
            .map_err(|error| Error::InvalidInput(error.to_string()))?;
        if metadata.file_type().is_symlink() {
            return Err(Error::InvalidInput(format!(
                "Node development headers contain symlink `{}`",
                entry.path().display()
            )));
        }
        if metadata.is_file() {
            let relative = entry
                .path()
                .strip_prefix(&root)
                .map_err(|_| Error::Corrupt("Node header walk escaped its root".to_string()))?
                .to_string_lossy()
                .into_owned();
            files.push((relative, entry.path().to_path_buf(), metadata.len()));
        }
    }
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let total = files.iter().try_fold(0u64, |total, (_, _, size)| {
        total
            .checked_add(*size)
            .ok_or_else(|| Error::InvalidInput("Node header size overflowed".to_string()))
    })?;
    if files.len() > MAX_FILES || total > MAX_BYTES {
        return Err(Error::InvalidInput(format!(
            "Node development headers exceed {MAX_FILES} files or {MAX_BYTES} bytes"
        )));
    }
    let mut identity = Vec::new();
    for (relative, path, size) in files {
        identity.extend_from_slice(relative.as_bytes());
        identity.push(0);
        identity.extend_from_slice(size.to_string().as_bytes());
        identity.push(0);
        identity.extend_from_slice(sha256_hex(&fs::read(path)?).as_bytes());
        identity.push(0);
    }
    Ok(sha256_hex(&identity))
}

fn join_repo_path(root: &str, name: &str) -> String {
    if root.is_empty() {
        name.to_string()
    } else {
        format!("{root}/{name}")
    }
}

fn strip_package_root(path: &str, package_root: &str) -> Result<String> {
    if package_root.is_empty() {
        return normalize_relative_path(path);
    }
    path.strip_prefix(&format!("{package_root}/"))
        .ok_or_else(|| Error::InvalidPath {
            path: path.to_string(),
            reason: format!("path is outside Node package root `{package_root}`"),
        })
        .and_then(normalize_relative_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn native_cow_mode() -> LaneWorkdirMode {
        if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        }
    }

    #[test]
    fn discovery_collapses_locked_workspace_descendants_but_keeps_nested_locks() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("package.json"),
            r#"{"name":"root","private":true}"#,
        )
        .unwrap();
        fs::write(
            workspace.path().join("pnpm-lock.yaml"),
            "lockfileVersion: '9.0'\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("pnpm-workspace.yaml"),
            "packages:\n  - packages/*\n",
        )
        .unwrap();
        fs::create_dir_all(workspace.path().join("packages/member/example")).unwrap();
        fs::write(
            workspace.path().join("packages/member/package.json"),
            r#"{"name":"member"}"#,
        )
        .unwrap();
        fs::write(
            workspace
                .path()
                .join("packages/member/example/package.json"),
            r#"{"name":"example"}"#,
        )
        .unwrap();
        fs::create_dir_all(workspace.path().join("standalone")).unwrap();
        fs::write(
            workspace.path().join("standalone/package.json"),
            r#"{"name":"standalone"}"#,
        )
        .unwrap();
        fs::write(
            workspace.path().join("standalone/package-lock.json"),
            r#"{"name":"standalone","lockfileVersion":3,"packages":{}}"#,
        )
        .unwrap();

        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "node-workspace",
            Some("main"),
            native_cow_mode(),
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let discovered = db
            .discover_workspace_environment("node-workspace", None)
            .unwrap();
        assert_eq!(
            discovered
                .components
                .iter()
                .filter(|component| component.adapter_identity == "trail/node@1")
                .map(|component| component.component_id.as_str())
                .collect::<Vec<_>>(),
            ["node", "node:standalone"]
        );
        if resolve_workspace_tool_executable("node").is_ok()
            && resolve_workspace_tool_executable("pnpm").is_ok()
        {
            let error = db
                .plan_workspace_environment("node-workspace", "node", None)
                .unwrap_err();
            assert!(error.to_string().contains("is a pnpm workspace root"));
        }
    }

    #[test]
    fn package_manager_specific_snapshot_formats_are_distinct_and_validated() {
        let cases = [
            (
                NodeResolutionSpec {
                    manager: "npm",
                    lock_name: "package-lock.json",
                },
                br#"{"name":"fixture","lockfileVersion":3,"packages":{}}"#.as_slice(),
            ),
            (
                NodeResolutionSpec {
                    manager: "pnpm",
                    lock_name: "pnpm-lock.yaml",
                },
                b"lockfileVersion: '9.0'\nimporters: {}\n".as_slice(),
            ),
            (
                NodeResolutionSpec {
                    manager: "yarn",
                    lock_name: "yarn.lock",
                },
                b"# yarn lockfile v1\n".as_slice(),
            ),
            (
                NodeResolutionSpec {
                    manager: "bun",
                    lock_name: "bun.lock",
                },
                b"{\n  \"lockfileVersion\": 1\n}\n".as_slice(),
            ),
        ];
        let mut formats = BTreeSet::new();
        for (spec, bytes) in cases {
            validate_node_lock_snapshot(&spec, bytes).unwrap();
            assert!(formats.insert(spec.snapshot_format()));
            assert_eq!(node_lock_resolution_argv(&spec)[0], spec.manager);
        }
        assert_eq!(formats.len(), 4);
        assert!(validate_node_lock_snapshot(
            &NodeResolutionSpec {
                manager: "npm",
                lock_name: "package-lock.json"
            },
            b"{}"
        )
        .unwrap_err()
        .to_string()
        .contains("lockfileVersion"));
    }

    #[test]
    fn yarn_and_bun_plans_bind_frozen_manager_specific_contracts() {
        if resolve_workspace_tool_executable("node").is_err() {
            return;
        }
        let cases = [
            (
                "yarn",
                "yarn.lock",
                "# yarn lockfile v1\n",
                "yarn-frozen-ignore-scripts-v1",
                "YARN_CACHE_FOLDER",
                "yarn",
                "--non-interactive",
            ),
            (
                "bun",
                "bun.lock",
                "{\n  \"lockfileVersion\": 1\n}\n",
                "bun-frozen-ignore-scripts-v1",
                "BUN_INSTALL_CACHE_DIR",
                "bun",
                "--no-progress",
            ),
        ];
        for (
            manager,
            lock_name,
            lock_contents,
            strategy,
            cache_environment,
            cache_relative,
            manager_flag,
        ) in cases
        {
            if resolve_workspace_tool_executable(manager).is_err() {
                continue;
            }
            let workspace = tempfile::tempdir().unwrap();
            let manager_version = tool_version(manager).unwrap();
            fs::write(
                workspace.path().join("package.json"),
                format!(
                    r#"{{"name":"trail-{manager}-plan","version":"1.0.0","private":true,"packageManager":"{manager}@{manager_version}"}}"#
                ),
            )
            .unwrap();
            fs::write(workspace.path().join(lock_name), lock_contents).unwrap();
            if manager == "bun" {
                fs::write(
                    workspace.path().join("bunfig.toml"),
                    "[install]\nignoreScripts = true\n",
                )
                .unwrap();
            }
            Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let db = Trail::open(workspace.path()).unwrap();
            let source_root = db.get_ref("refs/branches/main").unwrap().root_id;
            let plan = db.node_environment_plan(&source_root, "").unwrap();
            assert_eq!(plan.layer_key.strategy, strategy);
            assert_eq!(
                plan.layer_key.tool_versions[manager], manager_version,
                "manager identity must be exact"
            );
            let command = plan.command.as_ref().unwrap();
            assert!(command.args.iter().any(|arg| arg == "--frozen-lockfile"));
            assert!(command.args.iter().any(|arg| arg == "--ignore-scripts"));
            assert!(command.args.iter().any(|arg| arg == manager_flag));
            assert_eq!(
                Path::new(&command.environment[cache_environment]),
                plan.caches[0].storage_path.join(cache_relative)
            );
            assert_eq!(
                plan.outputs[0].policy,
                WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate
            );
            if manager == "bun" {
                assert!(plan
                    .inputs
                    .iter()
                    .any(|input| input.source_path == "bunfig.toml"));
            }
        }
    }

    #[test]
    fn yarn_pnp_and_ambiguous_manager_authority_fail_during_discovery() {
        let cases = [
            (
                r#"{"name":"pnp","packageManager":"yarn@1.22.22","installConfig":{"pnp":true}}"#,
                vec![("yarn.lock", "# yarn lockfile v1\n")],
                None,
                "Plug'n'Play",
            ),
            (
                r#"{"name":"berry","packageManager":"yarn@4.9.2"}"#,
                vec![("yarn.lock", "__metadata:\n  version: 8\n")],
                Some((".yarnrc.yml", "nodeLinker: pnp\n")),
                "Berry/PnP",
            ),
            (
                r#"{"name":"ambiguous"}"#,
                vec![
                    ("yarn.lock", "# yarn lockfile v1\n"),
                    (
                        "package-lock.json",
                        r#"{"name":"ambiguous","lockfileVersion":3,"packages":{}}"#,
                    ),
                ],
                None,
                "multiple package managers",
            ),
        ];
        for (package, locks, config, expected) in cases {
            let workspace = tempfile::tempdir().unwrap();
            fs::write(workspace.path().join("package.json"), package).unwrap();
            for (name, contents) in locks {
                fs::write(workspace.path().join(name), contents).unwrap();
            }
            if let Some((name, contents)) = config {
                fs::write(workspace.path().join(name), contents).unwrap();
            }
            Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let db = Trail::open(workspace.path()).unwrap();
            let source_root = db.get_ref("refs/branches/main").unwrap().root_id;
            let proposal = NODE_WORKSPACE_ADAPTER
                .propose(&db, &source_root, "")
                .unwrap()
                .unwrap();
            assert_eq!(proposal.status, EnvironmentComponentProposalStatus::Blocked);
            assert!(
                proposal.reasons[0].message.contains(expected),
                "unexpected proposal: {proposal:?}"
            );
        }
    }

    #[test]
    fn package_manager_declaration_must_match_lock_and_executable() {
        let package: serde_json::Value = serde_json::from_str(
            r#"{"name":"fixture","packageManager":"bun@1.2.3+sha512.deadbeef"}"#,
        )
        .unwrap();
        assert!(
            validate_declared_package_manager_version(&package, "yarn", "1.22.22")
                .unwrap_err()
                .to_string()
                .contains("Trail selected")
        );
        assert!(
            validate_declared_package_manager_version(&package, "bun", "1.2.4")
                .unwrap_err()
                .to_string()
                .contains("requires `bun@1.2.3`")
        );
        validate_declared_package_manager_version(&package, "bun", "1.2.3").unwrap();
    }

    #[test]
    fn approved_native_lifecycle_is_exact_private_and_identity_bearing() {
        for tool in ["node", "npm", "cc", "python3", "make"] {
            if resolve_workspace_tool_executable(tool).is_err() {
                return;
            }
        }
        let workspace = tempfile::tempdir().unwrap();
        let npm_version = tool_version("npm").unwrap();
        let package = format!(
            r#"{{"name":"trail-native-addon","version":"1.0.0","private":true,"packageManager":"npm@{npm_version}","scripts":{{"install":"node-gyp rebuild"}},"devDependencies":{{"node-gyp":"10.3.1"}}}}"#
        );
        let integrity = "sha512-dGVzdC1ub2RlLWd5cC1pbnRlZ3JpdHk=";
        let lock = format!(
            r#"{{"name":"trail-native-addon","version":"1.0.0","lockfileVersion":3,"requires":true,"packages":{{"":{{"name":"trail-native-addon","version":"1.0.0","hasInstallScript":true,"devDependencies":{{"node-gyp":"10.3.1"}}}},"node_modules/node-gyp":{{"version":"10.3.1","integrity":"{integrity}","dev":true,"bin":{{"node-gyp":"bin/node-gyp.js"}}}}}}}}"#
        );
        fs::write(workspace.path().join("package.json"), package).unwrap();
        fs::write(workspace.path().join("package-lock.json"), &lock).unwrap();
        fs::write(workspace.path().join("binding.gyp"), "{\"targets\":[]}").unwrap();
        let policy = serde_json::json!({
            "version": 1,
            "manager": format!("npm@{npm_version}"),
            "lock_sha256": sha256_hex(lock.as_bytes()),
            "packages": [format!("node-gyp@10.3.1#{integrity}")],
            "scripts": [{"package": ".", "phase": "install"}],
            "capabilities": {"network": "deny", "native_toolchain": true},
            "outputs": ["build"]
        });
        fs::write(
            workspace.path().join(NODE_LIFECYCLE_POLICY_FILE),
            serde_json::to_vec_pretty(&policy).unwrap(),
        )
        .unwrap();

        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let source_root = db.get_ref("refs/branches/main").unwrap().root_id;
        let plan = db.node_environment_plan(&source_root, "").unwrap();
        assert_eq!(
            plan.sandbox_policy,
            WorkspaceEnvironmentSandboxPolicy::ApprovedLifecycle
        );
        assert_eq!(
            plan.layer_key.strategy,
            "npm-frozen-approved-native-lifecycle-v1"
        );
        assert!(plan
            .layer_key
            .inputs
            .contains_key("lifecycle:approval_sha256"));
        assert!(plan.layer_key.tool_versions.contains_key("node-abi"));
        assert!(plan
            .layer_key
            .tool_versions
            .contains_key("native-compiler-executable"));
        assert_eq!(plan.pre_commands.len(), 1);
        assert!(plan.pre_commands[0]
            .args
            .iter()
            .any(|arg| arg == "--ignore-scripts"));
        assert_eq!(plan.command.as_ref().unwrap().args[0], "run-script");
        assert!(plan.outputs.iter().all(|output| {
            output.policy == WorkspaceEnvironmentOutputPolicy::WritablePrivate
                && output.scope == EnvironmentSharingScope::Lane
                && output.reuse == EnvironmentReuseMode::None
        }));
        assert_eq!(
            plan.source_projection,
            Some((source_root, "project".to_string()))
        );
    }

    #[test]
    fn approved_native_script_rejects_shell_and_transitive_lifecycle_escape() {
        for rejected in [
            "node-gyp rebuild && curl attacker.invalid",
            "npm rebuild",
            "node-gyp rebuild; npm run postinstall",
            "node-gyp rebuild ../../escape",
            "node-gyp configure",
        ] {
            assert!(
                !approved_node_native_script(rejected),
                "accepted `{rejected}`"
            );
        }
        for accepted in ["node-gyp rebuild", "node-gyp rebuild --debug"] {
            assert!(
                approved_node_native_script(accepted),
                "rejected `{accepted}`"
            );
        }
    }

    #[test]
    fn approved_native_lifecycle_never_runs_lock_pinned_transitive_scripts() {
        for tool in ["node", "npm", "cc", "python3", "make"] {
            if resolve_workspace_tool_executable(tool).is_err() {
                return;
            }
        }
        let workspace = tempfile::tempdir().unwrap();
        let npm_version = tool_version("npm").unwrap();
        let package = format!(
            r#"{{"name":"trail-native-addon","version":"1.0.0","private":true,"packageManager":"npm@{npm_version}","scripts":{{"install":"node-gyp rebuild"}},"devDependencies":{{"node-gyp":"10.3.1","hostile-transitive":"1.0.0"}}}}"#
        );
        let integrity = "sha512-dGVzdC1ub2RlLWd5cC1pbnRlZ3JpdHk=";
        let hostile_integrity = "sha512-aG9zdGlsZS10cmFuc2l0aXZlLWluc3RhbGw=";
        let lock = format!(
            r#"{{"name":"trail-native-addon","version":"1.0.0","lockfileVersion":3,"requires":true,"packages":{{"":{{"name":"trail-native-addon","version":"1.0.0","hasInstallScript":true,"devDependencies":{{"node-gyp":"10.3.1","hostile-transitive":"1.0.0"}}}},"node_modules/node-gyp":{{"version":"10.3.1","integrity":"{integrity}","dev":true,"bin":{{"node-gyp":"bin/node-gyp.js"}}}},"node_modules/hostile-transitive":{{"version":"1.0.0","integrity":"{hostile_integrity}","dev":true,"hasInstallScript":true}}}}}}"#
        );
        fs::write(workspace.path().join("package.json"), package).unwrap();
        fs::write(workspace.path().join("package-lock.json"), &lock).unwrap();
        fs::write(workspace.path().join("binding.gyp"), "{\"targets\":[]}").unwrap();
        let policy = serde_json::json!({
            "version": 1,
            "manager": format!("npm@{npm_version}"),
            "lock_sha256": sha256_hex(lock.as_bytes()),
            "packages": [format!("node-gyp@10.3.1#{integrity}")],
            "scripts": [{"package": ".", "phase": "install"}],
            "capabilities": {"network": "deny", "native_toolchain": true},
            "outputs": ["build"]
        });
        fs::write(
            workspace.path().join(NODE_LIFECYCLE_POLICY_FILE),
            serde_json::to_vec_pretty(&policy).unwrap(),
        )
        .unwrap();

        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let source_root = db.get_ref("refs/branches/main").unwrap().root_id;
        let plan = db.node_environment_plan(&source_root, "").unwrap();
        assert!(plan.pre_commands.iter().all(|command| {
            command
                .args
                .iter()
                .any(|argument| argument == "--ignore-scripts")
                && !command
                    .args
                    .iter()
                    .any(|argument| argument == "hostile-transitive")
        }));
        let lifecycle = plan.command.unwrap();
        assert_eq!(
            lifecycle.args,
            vec![
                "run-script".to_string(),
                "install".to_string(),
                "--ignore-scripts".to_string(),
                "--offline".to_string(),
            ]
        );
    }

    #[test]
    fn manifest_only_npm_uses_managed_lock_and_preserves_seed_cache_isolation() {
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
        let npm_version = tool_version("npm").unwrap();
        let package = format!(
            r#"{{"name":"trail-managed-node-lock","version":"1.0.0","private":true,"packageManager":"npm@{npm_version}"}}"#
        );
        fs::write(workspace.path().join("package.json"), &package).unwrap();

        let resolver = tempfile::tempdir().unwrap();
        fs::write(resolver.path().join("package.json"), &package).unwrap();
        let generated = Command::new("npm")
            .args([
                "install",
                "--package-lock-only",
                "--ignore-scripts",
                "--no-audit",
                "--no-fund",
            ])
            .current_dir(resolver.path())
            .output()
            .unwrap();
        assert!(
            generated.status.success(),
            "npm lock generation failed: {}",
            String::from_utf8_lossy(&generated.stderr)
        );
        let lock_bytes = fs::read(resolver.path().join("package-lock.json")).unwrap();

        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let source_root = db.get_ref("refs/branches/main").unwrap().root_id;
        assert!(NODE_WORKSPACE_ADAPTER
            .detect(&db, &source_root, "")
            .unwrap());
        assert_eq!(
            NODE_WORKSPACE_ADAPTER
                .propose(&db, &source_root, "")
                .unwrap()
                .unwrap()
                .status,
            EnvironmentComponentProposalStatus::Resolvable
        );
        let resolution_plan = NODE_WORKSPACE_ADAPTER
            .resolution_plan(&db, &source_root, "")
            .unwrap()
            .unwrap();
        assert_eq!(
            resolution_plan.snapshot_format,
            "node-lock-npm-package-lock.json-v1"
        );
        let resolved = db
            .resolve_artifact_component(
                ArtifactResolutionRequestV1 {
                    plan: resolution_plan,
                    candidate: ArtifactResolutionCandidateV1 {
                        snapshot_bytes: lock_bytes,
                        resolved_identities: BTreeMap::new(),
                        checksums: BTreeMap::new(),
                        contacted_authorities: Vec::new(),
                        stdout: Vec::new(),
                        stderr: Vec::new(),
                        redactions: Vec::new(),
                    },
                },
                false,
            )
            .unwrap();
        assert_eq!(resolved.decision, ArtifactResolutionDecisionV1::Resolved);

        for lane in ["managed-node-one", "managed-node-two"] {
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                lane,
                Some("main"),
                native_cow_mode(),
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
        }
        let first = db
            .sync_workspace_environment("managed-node-one", "node", None)
            .unwrap();
        let second = db
            .sync_workspace_environment("managed-node-two", "node", None)
            .unwrap();
        assert_eq!(first.layer_id, second.layer_id);
        assert_eq!(first.cache_key, second.cache_key);
        assert!(!workspace.path().join("package-lock.json").exists());
        assert!(db
            .root_file_entry(&source_root, "package-lock.json")
            .unwrap()
            .is_none());
        let generation = db
            .active_environment_generation("managed-node-one")
            .unwrap()
            .unwrap();
        assert_eq!(
            generation.components[0].outputs[0].policy,
            EnvironmentOutputPolicy::ImmutableSeedPrivate
        );
        assert_eq!(
            generation.components[0].caches[0].authority,
            "performance_only"
        );
        assert_eq!(generation.components[0].caches[0].protocol, "content_store");

        fs::write(workspace.path().join("README.md"), "new source root\n").unwrap();
        db.record(
            Some("main"),
            Some("change Node source root".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "managed-node-new-root",
            Some("main"),
            native_cow_mode(),
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let discovery = db
            .discover_workspace_environment("managed-node-new-root", None)
            .unwrap();
        assert_eq!(
            discovery.components[0].status,
            EnvironmentComponentProposalStatus::Resolvable
        );
    }

    #[test]
    fn two_lanes_with_identical_node_inputs_reuse_one_real_frozen_install() {
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
            r#"{"name":"trail-node-layer-test","version":"1.0.0","private":true}"#,
        )
        .unwrap();
        fs::write(
            workspace.path().join("package-lock.json"),
            r#"{"name":"trail-node-layer-test","version":"1.0.0","lockfileVersion":3,"requires":true,"packages":{"":{"name":"trail-node-layer-test","version":"1.0.0"}}}"#,
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        for lane in ["node-one", "node-two"] {
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                lane,
                Some("main"),
                native_cow_mode(),
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
        }
        let first = db.sync_node_dependencies("node-one", None).unwrap();
        let second = db
            .sync_workspace_environment("node-two", "auto", None)
            .unwrap();
        assert_eq!(first.layer_id, second.layer_id);
        assert_eq!(first.cache_key, second.cache_key);
        assert_eq!(db.list_workspace_layers().unwrap().len(), 1);
        let view_one = db.lane_workspace_view("node-one").unwrap().unwrap();
        let view_two = db.lane_workspace_view("node-two").unwrap().unwrap();
        let bound = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM workspace_view_layers WHERE layer_id = ?1 AND view_id IN (?2, ?3)",
                params![first.layer_id, view_one.view_id, view_two.view_id],
                |row| row.get::<_, i64>(0),
            )
            .unwrap();
        assert_eq!(bound, 2);
        let generation_one = db
            .active_environment_generation("node-one")
            .unwrap()
            .unwrap();
        let generation_two = db
            .active_environment_generation("node-two")
            .unwrap()
            .unwrap();
        let cache_one = &generation_one.components[0].caches[0];
        let cache_two = &generation_two.components[0].caches[0];
        assert_eq!(cache_one.namespace_id, cache_two.namespace_id);
        assert_eq!(cache_one.protocol, "content_store");
        assert_eq!(cache_one.access, "tool_concurrent");
        assert_eq!(cache_one.authority, "performance_only");
        let command_environment = db
            .lane_workspace_environment("node-one")
            .unwrap()
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        let cache_root = db
            .db_dir
            .join("cache/namespaces")
            .join(&cache_one.namespace_id);
        for (name, relative) in [
            ("npm_config_cache", "npm"),
            ("PNPM_HOME", "pnpm-home"),
            ("PNPM_STORE_DIR", "pnpm-store"),
            ("YARN_CACHE_FOLDER", "yarn"),
            ("BUN_INSTALL_CACHE_DIR", "bun"),
        ] {
            assert_eq!(
                Path::new(&command_environment[name]),
                cache_root.join(relative)
            );
        }
        let direct_node_modules = Path::new(&view_one.generated_upper).join("node_modules");
        assert_eq!(
            Path::new(&command_environment["TRAIL_NODE_MODULES"]),
            direct_node_modules
        );
        assert_eq!(
            Path::new(&command_environment["NODE_PATH"]),
            direct_node_modules
        );
        assert_eq!(
            std::env::split_paths(std::ffi::OsStr::new(&command_environment["PATH"]))
                .next()
                .unwrap(),
            direct_node_modules.join(".bin")
        );
        assert!(Path::new(&command_environment["TRAIL_NODE"]).is_absolute());
        assert!(db
            .db_dir
            .join("cache/namespaces")
            .join(&cache_one.namespace_id)
            .is_dir());
        assert!(!db.db_dir.join("cache/tool-home/node").exists());

        db.conn
            .execute(
                "UPDATE workspace_views SET owner_pid = ?1, owner_start_token = ?2, status = 'mounted' WHERE view_id = ?3",
                params![
                    std::process::id(),
                    current_process_start_token(),
                    view_one.view_id
                ],
            )
            .unwrap();
        let mounted = db.sync_node_dependencies("node-one", None).unwrap_err();
        assert!(mounted.to_string().contains("trail lane unmount node-one"));
        assert_eq!(
            db.workspace_environment_rows("node-one").unwrap()[0].status,
            "ready"
        );
        db.conn
            .execute(
                "UPDATE workspace_views SET owner_pid = NULL, owner_start_token = NULL, status = 'unmounted' WHERE view_id = ?1",
                params![view_one.view_id],
            )
            .unwrap();

        db.conn
            .execute(
                "UPDATE workspace_environment_states SET status = 'building', reason = 'sentinel', updated_at = -1 WHERE view_id = ?1",
                params![view_one.view_id],
            )
            .unwrap();
        let dynamic = db.workspace_environment_status("node-one").unwrap();
        assert_eq!(dynamic[0].status, "building");
        let persisted = db.workspace_environment_rows("node-one").unwrap().remove(0);
        assert_eq!(persisted.status, "building");
        assert_eq!(persisted.reason.as_deref(), Some("sentinel"));

        let normalized = db
            .enforce_read_only_mcp_call("trail.env_status", |db| {
                db.environment_component_status("node-one")
            })
            .unwrap();
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].component.component_id, "node");

        let normalized = db
            .enforce_read_only_mcp_call("trail.env_status", |db| {
                db.environment_component_status("node-one")
            })
            .unwrap();
        assert_eq!(normalized.len(), 1);
        assert_eq!(normalized[0].component.component_id, "node");
        assert_eq!(persisted.updated_at, -1);

        let paths = db.workspace_view_paths_for_lane("node-one").unwrap();
        let mut intent =
            super::super::workdir::ViewMutationJournal::open(&paths.source_upper).unwrap();
        intent
            .append(
                super::super::workdir::ViewMutationKind::Write,
                "package-lock.json",
                None,
            )
            .unwrap();
        fs::write(
            paths.source_upper.join("package-lock.json"),
            r#"{"name":"trail-node-layer-test","version":"1.0.1","lockfileVersion":3,"requires":true,"packages":{"":{"name":"trail-node-layer-test","version":"1.0.1"}}}"#,
        )
        .unwrap();
        db.checkpoint_lane_workspace("node-one", Some("lock changed".to_string()))
            .unwrap();
        let readiness = db.lane_readiness("node-one").unwrap();
        assert!(readiness
            .blockers
            .iter()
            .any(|issue| issue.code == "dependency_environment_stale"));
        let explanation = db
            .explain_workspace_environment_staleness("node-one", "node")
            .unwrap();
        assert!(explanation.complete);
        assert_eq!(explanation.status, "stale");
        assert!(explanation.changes.iter().any(|change| {
            change.dimension == "input"
                && change.name == "package-lock.json"
                && change.change == "modified"
        }));
        let state = db.environment_component_status("node-one").unwrap();
        assert!(state[0]
            .reason
            .as_deref()
            .is_some_and(|reason| reason.contains("input:package-lock.json modified")));
    }
}
