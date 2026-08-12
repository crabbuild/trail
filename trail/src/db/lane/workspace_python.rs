use super::workspace_environment::{
    mounted_output_placeholder, mounted_output_relative_placeholder,
    mounted_resolution_input_placeholder, resolve_workspace_tool_executable,
    workspace_mounted_commands_identity, WorkspaceEnvironmentAdapter,
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

pub(crate) struct PythonVenvAdapter;

pub(crate) static PYTHON_VENV_ADAPTER: PythonVenvAdapter = PythonVenvAdapter;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PythonInstallContract {
    UvLock,
    HashedRequirements,
    ManagedHashedRequirements,
}

const PYTHON_IDENTITY_FILES: [&str; 8] = [
    "pyproject.toml",
    ".python-version",
    "uv.lock",
    "poetry.lock",
    "pdm.lock",
    "Pipfile.lock",
    "requirements.lock",
    "requirements.txt",
];

const PYTHON_RESOLUTION_FILES: [&str; 6] = [
    "uv.lock",
    "poetry.lock",
    "pdm.lock",
    "Pipfile.lock",
    "requirements.lock",
    "requirements.txt",
];

static PYTHON_VENV_ADAPTER_METADATA: WorkspaceEnvironmentAdapterMetadata =
    WorkspaceEnvironmentAdapterMetadata {
        canonical_identity: "trail/python-venv@1",
        namespace: "trail",
        name: "python-venv",
        contract_major: 1,
        implementation_version: env!("CARGO_PKG_VERSION"),
        distribution_digest: "builtin:python-venv-plan-v6",
        selectors: &["trail/python-venv@1", "python-venv", "python"],
        kind: "dependency",
        layer_adapter_name: "python-venv",
        discovery_markers: &PYTHON_IDENTITY_FILES,
        supported_operating_systems: &["linux", "macos", "windows"],
        supported_architectures: &["aarch64", "x86_64"],
        stability: "experimental",
        description: "Automatically initialized lane-private Python virtual environment with direct command bindings",
    };

const PYTHON_CACHE_COMMAND_BINDINGS: &[WorkspaceEnvironmentCacheCommandBinding] = &[
    WorkspaceEnvironmentCacheCommandBinding {
        cache_name: "python-downloads",
        environment: "PIP_CACHE_DIR",
        relative_path: "pip",
        required: true,
    },
    WorkspaceEnvironmentCacheCommandBinding {
        cache_name: "python-downloads",
        environment: "UV_CACHE_DIR",
        relative_path: "uv",
        required: true,
    },
];

#[cfg(windows)]
const PYTHON_VENV_EXECUTABLE_DIRECTORY: &str = "Scripts";
#[cfg(not(windows))]
const PYTHON_VENV_EXECUTABLE_DIRECTORY: &str = "bin";
#[cfg(windows)]
const PYTHON_VENV_EXECUTABLE: &str = "Scripts/python.exe";
#[cfg(not(windows))]
const PYTHON_VENV_EXECUTABLE: &str = "bin/python";

const PYTHON_OUTPUT_COMMAND_BINDINGS: &[WorkspaceEnvironmentOutputCommandBinding] = &[
    WorkspaceEnvironmentOutputCommandBinding {
        output_name: "venv",
        environment: Some("VIRTUAL_ENV"),
        relative_path: "",
        direct: true,
        prepend_path: false,
        required: true,
    },
    WorkspaceEnvironmentOutputCommandBinding {
        output_name: "venv",
        environment: None,
        relative_path: PYTHON_VENV_EXECUTABLE_DIRECTORY,
        direct: true,
        prepend_path: true,
        required: true,
    },
    WorkspaceEnvironmentOutputCommandBinding {
        output_name: "venv",
        environment: Some("TRAIL_VENV_PYTHON"),
        relative_path: PYTHON_VENV_EXECUTABLE,
        direct: true,
        prepend_path: false,
        required: true,
    },
];

const PYTHON_TOOL_COMMAND_BINDINGS: &[WorkspaceEnvironmentToolCommandBinding] =
    &[WorkspaceEnvironmentToolCommandBinding {
        programs: &["python3", "python"],
        environment: "TRAIL_PYTHON",
        required: true,
        prepend_path: false,
    }];

impl WorkspaceEnvironmentAdapter for PythonVenvAdapter {
    fn metadata(&self) -> &'static WorkspaceEnvironmentAdapterMetadata {
        &PYTHON_VENV_ADAPTER_METADATA
    }

    fn component_id(&self, component_root: &str) -> Result<String> {
        let root = normalize_python_component_root(component_root)?;
        Ok(if root.is_empty() {
            "python-venv".to_string()
        } else {
            format!("python-venv:{root}")
        })
    }

    fn cache_command_bindings(&self) -> &'static [WorkspaceEnvironmentCacheCommandBinding] {
        PYTHON_CACHE_COMMAND_BINDINGS
    }

    fn output_command_bindings(&self) -> &'static [WorkspaceEnvironmentOutputCommandBinding] {
        PYTHON_OUTPUT_COMMAND_BINDINGS
    }

    fn tool_command_bindings(&self) -> &'static [WorkspaceEnvironmentToolCommandBinding] {
        PYTHON_TOOL_COMMAND_BINDINGS
    }

    fn detect(&self, db: &Trail, source_root: &ObjectId, component_root: &str) -> Result<bool> {
        let root = normalize_python_component_root(component_root)?;
        for file in PYTHON_IDENTITY_FILES {
            if db
                .root_file_entry(source_root, &join_python_path(&root, file))?
                .is_some()
            {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn propose(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<Option<WorkspaceEnvironmentAdapterProposal>> {
        let root = normalize_python_component_root(component_root)?;
        if !self.detect(db, source_root, &root)? {
            return Ok(None);
        }
        Ok(Some(WorkspaceEnvironmentAdapterProposal::ready()))
    }

    fn resolution_plan(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<Option<ArtifactResolutionPlanV1>> {
        let root = normalize_python_component_root(component_root)?;
        let pyproject_path = join_python_path(&root, "pyproject.toml");
        let Some(pyproject) = db.root_file_entry(source_root, &pyproject_path)? else {
            return Ok(None);
        };
        let uv = resolve_workspace_tool_executable("uv")?;
        let policy_identity = sha256_hex(
            format!(
                "python-uv-requirements-resolver-v1\0{}\0offline\0generate-hashes",
                uv.identity
            )
            .as_bytes(),
        );
        Ok(Some(ArtifactResolutionPlanV1 {
            version: ARTIFACT_RESOLUTION_PLAN_VERSION,
            proposal_key: python_resolution_proposal_key(source_root, &root),
            source_root: source_root.clone(),
            component_id: self.component_id(&root)?,
            adapter_identity: self.identity().to_string(),
            policy_identity,
            program: "uv".to_string(),
            resolved_program: uv.path.to_string_lossy().into_owned(),
            executable_identity: uv.identity,
            argv: vec![
                "uv".to_string(),
                "pip".to_string(),
                "compile".to_string(),
                "--offline".to_string(),
                "--generate-hashes".to_string(),
                "--output-file".to_string(),
                "requirements.lock".to_string(),
                "pyproject.toml".to_string(),
            ],
            working_directory: if root.is_empty() {
                ".".to_string()
            } else {
                root.clone()
            },
            readable_inputs: vec![ArtifactResolutionInputV1 {
                source_path: pyproject_path,
                content_hash: pyproject.content_hash,
                size_bytes: pyproject.size_bytes,
            }],
            candidate_output: join_python_path(&root, "requirements.lock"),
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
            snapshot_format: "python-requirements-hashes-v1".to_string(),
            validations: vec![ArtifactValidationV1 {
                name: "python-requirements-hashes".to_string(),
                kind: ArtifactValidationKindV1::Framework,
                required: true,
                parameters: BTreeMap::from([(
                    "hash_mode".to_string(),
                    "required-for-nonempty".to_string(),
                )]),
            }],
        }))
    }

    fn plan(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<WorkspaceEnvironmentPlan> {
        let component_root = normalize_python_component_root(component_root)?;
        let python = resolve_python_executable_for_source(db, source_root, &component_root)?;
        let component_id = self.component_id(&component_root)?;
        let mount_path = join_python_path(&component_root, ".venv");
        let implementation_version = env!("CARGO_PKG_VERSION").to_string();
        let distribution_digest = "builtin:python-venv-plan-v6".to_string();
        let source_resolution = python_source_resolution(db, source_root, &component_root)?;
        let managed_snapshot = if source_resolution.is_none() {
            python_resolution_snapshot(db, source_root, &component_root)?
        } else {
            None
        };
        let mut mounted_args = vec![
            "-m".to_string(),
            "venv".to_string(),
            "--without-pip".to_string(),
        ];
        // Python otherwise attempts interpreter symlinks first. macOS NFS
        // clients can reject that operation and make venv print a warning
        // before falling back to a copy, so request the supported copy mode
        // directly for a clean first-run experience.
        #[cfg(target_os = "macos")]
        mounted_args.push("--copies".to_string());
        mounted_args.push(mounted_output_placeholder("venv"));
        let venv_command = WorkspaceEnvironmentCommand {
            program: "python".to_string(),
            resolved_program: python.path.clone(),
            executable_identity: python.identity.clone(),
            args: mounted_args,
            working_directory: component_root.clone(),
            environment: BTreeMap::new(),
            remove_environment: Vec::new(),
            cache_names: Vec::new(),
        };
        let mut key_inputs = BTreeMap::from([
            ("component_id".to_string(), component_id.clone()),
            ("component_root".to_string(), component_root.clone()),
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
                format!("writable-private:{mount_path}"),
            ),
            (
                "creation_phase".to_string(),
                "host-mounted-initialization".to_string(),
            ),
            (
                "command_environment".to_string(),
                format!(
                    "PIP_CACHE_DIR=cache:python-downloads/pip;UV_CACHE_DIR=cache:python-downloads/uv;VIRTUAL_ENV=direct:venv;TRAIL_VENV_PYTHON=direct:venv/{PYTHON_VENV_EXECUTABLE};PATH+=direct:venv/{PYTHON_VENV_EXECUTABLE_DIRECTORY};TRAIL_PYTHON=tool:python3|python"
                ),
            ),
        ]);
        let mut resolution_inputs = Vec::new();
        let mut source_projection = None;
        let managed_resolution = managed_snapshot.is_some();
        let install_contract = match source_resolution.as_deref() {
            Some(path) if path.ends_with("uv.lock") => Some(PythonInstallContract::UvLock),
            Some(path) if path.ends_with("requirements.lock") => {
                let entry = db.root_file_entry(source_root, path)?.ok_or_else(|| {
                    Error::Corrupt(format!("Python resolution input `{path}` disappeared"))
                })?;
                validate_python_requirements_snapshot(&db.materialize_entry_bytes(&entry)?)?;
                Some(PythonInstallContract::HashedRequirements)
            }
            Some(path) => {
                return Err(Error::InvalidInput(format!(
                    "Python component `{}` uses `{path}`, which is not a frozen install contract; provide uv.lock or a hash-pinned requirements.lock",
                    display_python_root(&component_root)
                )));
            }
            None if managed_resolution => Some(PythonInstallContract::ManagedHashedRequirements),
            None => None,
        };
        let uv = install_contract
            .map(|_| resolve_workspace_tool_executable("uv"))
            .transpose()?;
        if let Some((snapshot_id, snapshot, bytes)) = managed_snapshot {
            let resolution_plan = self
                .resolution_plan(db, source_root, &component_root)?
                .ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "Python component `{}` has no supported source lock and cannot produce a managed resolution plan",
                        display_python_root(&component_root)
                    ))
                })?;
            if snapshot.resolver_executable_identity != resolution_plan.executable_identity
                || snapshot.policy_identity != resolution_plan.policy_identity
            {
                return Err(Error::InvalidInput(format!(
                    "Python component `{}` resolution snapshot was created by a different uv executable or resolver policy; resolve it again for the current tool",
                    display_python_root(&component_root)
                )));
            }
            let resolution_path = join_python_path(&component_root, "requirements.lock");
            let size_bytes = u64::try_from(bytes.len()).map_err(|_| {
                Error::InvalidInput(
                    "Python resolution snapshot exceeds platform limits".to_string(),
                )
            })?;
            key_inputs.insert("source_root".to_string(), source_root.0.clone());
            key_inputs.insert(
                format!("input:{resolution_path}"),
                snapshot.content_sha256.clone(),
            );
            key_inputs.insert(
                "resolution_authority".to_string(),
                format!("snapshot:{}", snapshot_id.0),
            );
            resolution_inputs.push(WorkspaceEnvironmentResolutionInput {
                snapshot_id,
                source_root: source_root.clone(),
                source_path: resolution_path.clone(),
                staging_path: format!("project/{resolution_path}"),
                content_hash: snapshot.content_sha256,
                size_bytes,
            });
            source_projection = Some((source_root.clone(), "project".to_string()));
        }
        let mut inputs = Vec::new();
        for file in PYTHON_IDENTITY_FILES {
            let path = join_python_path(&component_root, file);
            if let Some(entry) = db.root_file_entry(source_root, &path)? {
                key_inputs.insert(format!("input:{path}"), entry.content_hash.clone());
                if !managed_resolution {
                    inputs.push(WorkspaceEnvironmentInput {
                        source_path: path.clone(),
                        staging_path: format!("project/{path}"),
                        entry,
                    });
                }
            }
        }
        if inputs.is_empty() && resolution_inputs.is_empty() {
            return Err(Error::InvalidInput(format!(
                "Python component `{}` has no supported dependency manifest or lockfile",
                display_python_root(&component_root)
            )));
        }
        inputs.sort_by(|left, right| left.source_path.cmp(&right.source_path));
        let download_cache = db.declare_workspace_environment_cache(
            self.identity(),
            "python-downloads",
            WorkspaceEnvironmentCacheProtocol::ContentStore,
            WorkspaceEnvironmentCacheAccess::ToolConcurrent,
            BTreeMap::from([
                ("python_executable".to_string(), python.identity.clone()),
                ("platform".to_string(), std::env::consts::OS.to_string()),
                (
                    "architecture".to_string(),
                    std::env::consts::ARCH.to_string(),
                ),
            ]),
        )?;
        let pre_commands = if managed_resolution {
            let working_directory = if component_root.is_empty() {
                "project".to_string()
            } else {
                format!("project/{component_root}")
            };
            vec![WorkspaceEnvironmentCommand {
                program: "python".to_string(),
                resolved_program: python.path.clone(),
                executable_identity: python.identity.clone(),
                args: vec![
                    "-m".to_string(),
                    "pip".to_string(),
                    "download".to_string(),
                    "--require-hashes".to_string(),
                    "--no-deps".to_string(),
                    "--disable-pip-version-check".to_string(),
                    "--dest".to_string(),
                    download_cache
                        .storage_path
                        .join("wheels")
                        .to_string_lossy()
                        .into_owned(),
                    "-r".to_string(),
                    "requirements.lock".to_string(),
                ],
                working_directory,
                environment: BTreeMap::from([
                    (
                        "PIP_CACHE_DIR".to_string(),
                        download_cache
                            .storage_path
                            .join("pip")
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    (
                        "UV_CACHE_DIR".to_string(),
                        download_cache
                            .storage_path
                            .join("uv")
                            .to_string_lossy()
                            .into_owned(),
                    ),
                ]),
                remove_environment: Vec::new(),
                cache_names: vec![download_cache.name.clone()],
            }]
        } else {
            Vec::new()
        };
        let mut mounted_commands = vec![venv_command];
        if let (Some(contract), Some(uv)) = (install_contract, uv.as_ref()) {
            let venv_python = mounted_output_relative_placeholder("venv", PYTHON_VENV_EXECUTABLE);
            let mut args = match contract {
                PythonInstallContract::UvLock => vec![
                    "sync".to_string(),
                    "--frozen".to_string(),
                    "--no-install-project".to_string(),
                    "--no-python-downloads".to_string(),
                    "--active".to_string(),
                ],
                PythonInstallContract::HashedRequirements => vec![
                    "pip".to_string(),
                    "sync".to_string(),
                    "--require-hashes".to_string(),
                    "--python".to_string(),
                    venv_python,
                    "requirements.lock".to_string(),
                ],
                PythonInstallContract::ManagedHashedRequirements => vec![
                    "pip".to_string(),
                    "sync".to_string(),
                    "--require-hashes".to_string(),
                    "--offline".to_string(),
                    "--find-links".to_string(),
                    download_cache
                        .storage_path
                        .join("wheels")
                        .to_string_lossy()
                        .into_owned(),
                    "--python".to_string(),
                    venv_python,
                    mounted_resolution_input_placeholder(&join_python_path(
                        &component_root,
                        "requirements.lock",
                    )),
                ],
            };
            args.shrink_to_fit();
            mounted_commands.push(WorkspaceEnvironmentCommand {
                program: "uv".to_string(),
                resolved_program: uv.path.clone(),
                executable_identity: uv.identity.clone(),
                args,
                working_directory: component_root.clone(),
                environment: BTreeMap::from([
                    (
                        "PIP_CACHE_DIR".to_string(),
                        download_cache
                            .storage_path
                            .join("pip")
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    (
                        "UV_CACHE_DIR".to_string(),
                        download_cache
                            .storage_path
                            .join("uv")
                            .to_string_lossy()
                            .into_owned(),
                    ),
                    ("UV_NO_PROGRESS".to_string(), "1".to_string()),
                    ("UV_LINK_MODE".to_string(), "copy".to_string()),
                    ("UV_PYTHON_DOWNLOADS".to_string(), "never".to_string()),
                    (
                        "VIRTUAL_ENV".to_string(),
                        mounted_output_placeholder("venv"),
                    ),
                ]),
                remove_environment: Vec::new(),
                cache_names: Vec::new(),
            });
        }
        key_inputs.insert(
            "mounted_action".to_string(),
            workspace_mounted_commands_identity(&mounted_commands)?,
        );
        let mut tool_versions =
            BTreeMap::from([("python-executable".to_string(), python.identity.clone())]);
        if let Some(uv) = &uv {
            tool_versions.insert("uv-executable".to_string(), uv.identity.clone());
        }
        Ok(WorkspaceEnvironmentPlan {
            component_id,
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
                inputs: key_inputs,
                tool_versions,
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "lane-private-host-python".to_string(),
                strategy: "python-venv-private-direct-init-v5".to_string(),
            },
            inputs,
            resolution_inputs,
            construction_seed: None,
            source_projection,
            pre_commands,
            // Python virtual environments commonly embed absolute interpreter
            // and prefix paths. The host initializes the candidate's physical
            // private upper, then binds that exact path into managed commands;
            // the conventional `.venv` path remains visible in the lane view.
            command: None,
            mounted_commands,
            caches: vec![download_cache],
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
            outputs: vec![WorkspaceEnvironmentOutput {
                name: "venv".to_string(),
                output_path: "private/venv".to_string(),
                mount_path,
                policy: WorkspaceEnvironmentOutputPolicy::WritablePrivate,
                reuse: EnvironmentReuseMode::None,
                scope: EnvironmentSharingScope::Lane,
                publish: EnvironmentPublicationTrigger::Never,
                gate: None,
                create_if_missing: true,
            }],
            stale_reason:
                "Python executable, dependency manifest or lockfile, component root, platform, or adapter policy changed"
                    .to_string(),
        })
    }
}

fn python_source_resolution(
    db: &Trail,
    source_root: &ObjectId,
    component_root: &str,
) -> Result<Option<String>> {
    for file in PYTHON_RESOLUTION_FILES {
        let path = join_python_path(component_root, file);
        if db.root_file_entry(source_root, &path)?.is_some() {
            return Ok(Some(path));
        }
    }
    Ok(None)
}

fn python_resolution_proposal_key(source_root: &ObjectId, component_root: &str) -> String {
    let identity = format!(
        "python-requirements-resolution-v1\0{}\0{}\0{}",
        source_root.0, component_root, PYTHON_VENV_ADAPTER_METADATA.canonical_identity
    );
    format!("python_requirements_v1_{}", sha256_hex(identity.as_bytes()))
}

fn python_resolution_snapshot(
    db: &Trail,
    source_root: &ObjectId,
    component_root: &str,
) -> Result<Option<(ObjectId, ArtifactResolutionSnapshotV1, Vec<u8>)>> {
    let proposal_key = python_resolution_proposal_key(source_root, component_root);
    let expected_component = PYTHON_VENV_ADAPTER.component_id(component_root)?;
    db.verified_workspace_environment_resolution_snapshot(
        &proposal_key,
        source_root,
        &expected_component,
        PYTHON_VENV_ADAPTER.identity(),
        "python-requirements-hashes-v1",
        validate_python_requirements_snapshot,
    )
}

fn validate_python_requirements_snapshot(bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        Error::InvalidInput("Trail-managed Python requirements snapshot is not UTF-8".to_string())
    })?;
    let mut has_requirement = false;
    let mut current_requirement_hashed = true;
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(hash) = line.strip_prefix("--hash=sha256:") {
            let hash = hash.trim_end_matches('\\').trim();
            if !has_requirement
                || hash.len() != 64
                || !hash.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(Error::InvalidInput(
                    "Trail-managed Python requirements snapshot contains an invalid or unbound SHA-256 hash"
                        .to_string(),
                ));
            }
            current_requirement_hashed = true;
            continue;
        }
        if line.starts_with('-') || line.contains(" @ ") {
            return Err(Error::InvalidInput(
                "Trail-managed Python requirements snapshot contains an unpinned directive or URL"
                    .to_string(),
            ));
        }
        if has_requirement && !current_requirement_hashed {
            return Err(Error::InvalidInput(
                "Trail-managed Python requirements snapshot contains packages without SHA-256 hashes"
                    .to_string(),
            ));
        }
        has_requirement = true;
        if !line.contains("==") {
            return Err(Error::InvalidInput(
                "Trail-managed Python requirements snapshot contains an unpinned requirement"
                    .to_string(),
            ));
        }
        current_requirement_hashed = line
            .split_whitespace()
            .filter_map(|token| token.strip_prefix("--hash=sha256:"))
            .map(|hash| hash.trim_end_matches('\\'))
            .any(|hash| hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()));
    }
    if has_requirement && !current_requirement_hashed {
        return Err(Error::InvalidInput(
            "Trail-managed Python requirements snapshot contains packages without SHA-256 hashes"
                .to_string(),
        ));
    }
    Ok(())
}

fn resolve_python_executable() -> Result<super::workspace_environment::ResolvedWorkspaceTool> {
    #[cfg(windows)]
    let candidates = ["python", "python3"];
    #[cfg(not(windows))]
    let candidates = ["python3", "python"];
    let mut errors = Vec::new();
    for candidate in candidates {
        match resolve_workspace_tool_executable(candidate) {
            Ok(tool) => return Ok(tool),
            Err(error) => errors.push(error.to_string()),
        }
    }
    Err(Error::InvalidInput(format!(
        "Python adapter requires `python3` or `python` on PATH: {}",
        errors.join("; ")
    )))
}

fn resolve_python_executable_for_source(
    db: &Trail,
    source_root: &ObjectId,
    component_root: &str,
) -> Result<super::workspace_environment::ResolvedWorkspaceTool> {
    let version_path = join_python_path(component_root, ".python-version");
    let Some(entry) = db.root_file_entry(source_root, &version_path)? else {
        return resolve_python_executable();
    };
    let version_bytes = db.materialize_entry_bytes(&entry)?;
    let selector = python_version_selector(&version_bytes)?;
    let program = format!("python{selector}");
    resolve_workspace_tool_executable(&program).map_err(|error| {
        Error::InvalidInput(format!(
            "Python component `{}` pins `{selector}` in `{version_path}`, but `{program}` is unavailable on PATH: {error}",
            display_python_root(component_root)
        ))
    })
}

fn python_version_selector(bytes: &[u8]) -> Result<String> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::InvalidInput(".python-version must be UTF-8".to_string()))?;
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
    let raw = lines.next().ok_or_else(|| {
        Error::InvalidInput(".python-version must contain one Python version".to_string())
    })?;
    if lines.next().is_some()
        || raw.len() > 64
        || raw
            .chars()
            .any(|ch| !ch.is_ascii_alphanumeric() && !matches!(ch, '.' | '-' | '_'))
    {
        return Err(Error::InvalidInput(
            ".python-version must contain one bounded, portable Python version selector"
                .to_string(),
        ));
    }
    let numeric = raw.strip_prefix("cpython-").unwrap_or(raw);
    let mut parts = numeric.split('.');
    let major = parts.next().unwrap_or_default();
    let minor = parts.next().unwrap_or_default();
    if major.is_empty()
        || minor.is_empty()
        || !major.chars().all(|ch| ch.is_ascii_digit())
        || !minor.chars().all(|ch| ch.is_ascii_digit())
    {
        return Err(Error::InvalidInput(format!(
            "unsupported Python version selector `{raw}`; use a CPython major.minor or major.minor.patch version"
        )));
    }
    Ok(format!("{major}.{minor}"))
}

fn normalize_python_component_root(component_root: &str) -> Result<String> {
    if component_root.trim_matches('/').is_empty() {
        Ok(String::new())
    } else {
        normalize_relative_path(component_root)
    }
}

fn join_python_path(root: &str, child: &str) -> String {
    if root.is_empty() {
        child.to_string()
    } else {
        format!("{root}/{child}")
    }
}

fn display_python_root(root: &str) -> &str {
    if root.is_empty() {
        "."
    } else {
        root
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    use std::ffi::OsStr;

    fn wait_for_mounted_crash_handshake(
        child: &mut std::process::Child,
        ready: &Path,
        phase: &str,
    ) {
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while std::time::Instant::now() < deadline {
            if ready.is_file() {
                return;
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!("mounted crash helper exited at {phase} before handshake: {status}");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = child.kill();
        panic!("timed out waiting for mounted crash helper at {phase}");
    }

    #[test]
    fn python_requirements_snapshot_requires_pins_and_hashes() {
        validate_python_requirements_snapshot(
            b"# generated\nexample==1.2.3 --hash=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        )
        .unwrap();
        validate_python_requirements_snapshot(b"# no dependencies\n").unwrap();
        assert!(validate_python_requirements_snapshot(b"example>=1\n")
            .unwrap_err()
            .to_string()
            .contains("unpinned"));
        assert!(validate_python_requirements_snapshot(b"example==1\n")
            .unwrap_err()
            .to_string()
            .contains("without SHA-256"));
        assert!(validate_python_requirements_snapshot(
            b"first==1 --hash=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\nsecond==2\n"
        )
        .unwrap_err()
        .to_string()
        .contains("without SHA-256"));
        assert!(
            validate_python_requirements_snapshot(b"example==1 --hash=sha256:not-a-digest\n")
                .unwrap_err()
                .to_string()
                .contains("without SHA-256")
        );
        assert!(validate_python_requirements_snapshot(
            b"--hash=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n"
        )
        .unwrap_err()
        .to_string()
        .contains("invalid or unbound"));
        assert!(validate_python_requirements_snapshot(b"-e ../example\n")
            .unwrap_err()
            .to_string()
            .contains("directive or URL"));
    }

    #[test]
    fn python_version_file_selects_a_portable_major_minor_executable() {
        assert_eq!(python_version_selector(b"3.12\n").unwrap(), "3.12");
        assert_eq!(
            python_version_selector(b"cpython-3.13.2\n").unwrap(),
            "3.13"
        );
        assert!(python_version_selector(b"3.12\n3.13\n").is_err());
        assert!(python_version_selector(b"../python\n").is_err());
        assert!(python_version_selector(b"pypy-3.11\n").is_err());
    }

    #[test]
    fn python_plan_rejects_unfrozen_requirements_instead_of_creating_an_empty_venv() {
        if resolve_python_executable().is_err() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("pyproject.toml"),
            "[project]\nname = \"unfrozen\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(workspace.path().join("requirements.txt"), "pytest>=8\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "unfrozen",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let error = db
            .plan_workspace_environment("unfrozen", "python", None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("not a frozen install contract"), "{error}");
        assert!(error.contains("requirements.lock"), "{error}");
    }

    #[test]
    fn python_hashed_requirements_plan_uses_uv_hash_enforcement_and_direct_output() {
        if resolve_python_executable().is_err() || resolve_workspace_tool_executable("uv").is_err()
        {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("pyproject.toml"),
            "[project]\nname = \"hashed\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("requirements.lock"),
            "example==1 --hash=sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "hashed",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let plan = db
            .plan_workspace_environment("hashed", "python", None)
            .unwrap();
        let install = &plan.commands[1];
        assert_eq!(install.program, "uv");
        assert!(install.args.iter().any(|arg| arg == "--require-hashes"));
        assert!(install.args.iter().any(|arg| arg == "requirements.lock"));
        let expected_python = format!("{{trail-mounted-output:venv:{PYTHON_VENV_EXECUTABLE}}}");
        assert!(install.args.iter().any(|arg| arg == &expected_python));
    }

    #[test]
    fn managed_python_resolution_warms_download_cache_but_keeps_venv_and_bytecode_private() {
        let Ok(python) = resolve_python_executable() else {
            return;
        };
        let Ok(uv) = resolve_workspace_tool_executable("uv") else {
            return;
        };
        let workspace = tempfile::tempdir().unwrap();
        let pyproject =
            "[project]\nname = \"managed-python\"\nversion = \"0.1.0\"\ndependencies = []\n";
        fs::write(workspace.path().join("pyproject.toml"), pyproject).unwrap();
        let resolver = tempfile::tempdir().unwrap();
        fs::write(resolver.path().join("pyproject.toml"), pyproject).unwrap();
        let generated = Command::new(&uv.path)
            .args([
                "pip",
                "compile",
                "--offline",
                "--generate-hashes",
                "--output-file",
                "requirements.lock",
                "pyproject.toml",
            ])
            .current_dir(resolver.path())
            .output()
            .unwrap();
        assert!(
            generated.status.success(),
            "uv resolution failed: {}",
            String::from_utf8_lossy(&generated.stderr)
        );
        let requirements = fs::read(resolver.path().join("requirements.lock")).unwrap();
        validate_python_requirements_snapshot(&requirements).unwrap();

        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let source_root = db.get_ref("refs/branches/main").unwrap().root_id;
        let resolution_plan = PYTHON_VENV_ADAPTER
            .resolution_plan(&db, &source_root, "")
            .unwrap()
            .unwrap();
        assert_eq!(resolution_plan.executable_identity, uv.identity);
        db.resolve_artifact_component(
            ArtifactResolutionRequestV1 {
                plan: resolution_plan,
                candidate: ArtifactResolutionCandidateV1 {
                    snapshot_bytes: requirements,
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

        for lane in ["managed-python-one", "managed-python-two"] {
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
            .plan_workspace_environment("managed-python-one", "python", None)
            .unwrap();
        assert_eq!(plan.inputs.len(), 1);
        assert_eq!(plan.inputs[0].source_path, "requirements.lock");
        assert_eq!(plan.caches.len(), 1);
        assert_eq!(plan.caches[0].protocol, "content_store");
        assert_eq!(plan.caches[0].authority, "performance_only");
        assert_eq!(plan.commands.len(), 3);
        assert_eq!(plan.commands[0].phase, "staging");
        assert_eq!(plan.commands[1].phase, "mounted_initialization");
        assert_eq!(plan.commands[2].phase, "mounted_initialization");
        assert_eq!(plan.commands[2].program, "uv");
        assert!(!workspace.path().join("requirements.lock").exists());
        assert!(db
            .root_file_entry(&source_root, "requirements.lock")
            .unwrap()
            .is_none());

        #[cfg(target_os = "linux")]
        if std::env::var_os("TRAIL_RUN_FUSE_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        #[cfg(target_os = "macos")]
        if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        #[cfg(windows)]
        if std::env::var_os("TRAIL_RUN_DOKAN_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        let first = db
            .sync_workspace_environment_component("managed-python-one", "python", None, None)
            .unwrap();
        let second = db
            .sync_workspace_environment_component("managed-python-two", "python", None, None)
            .unwrap();
        assert!(first.layers.is_empty());
        assert!(second.layers.is_empty());
        assert_eq!(
            first.generation.components[0].caches[0].namespace_id,
            second.generation.components[0].caches[0].namespace_id
        );
        assert_eq!(first.generation.components[0].outputs[0].layer_id, None);
        let first_paths = db
            .workspace_view_paths_for_lane("managed-python-one")
            .unwrap();
        let second_paths = db
            .workspace_view_paths_for_lane("managed-python-two")
            .unwrap();
        let first_bytecode = first_paths
            .generated_upper
            .join(".venv/lib/private/__pycache__");
        fs::create_dir_all(&first_bytecode).unwrap();
        fs::write(first_bytecode.join("module.pyc"), b"private bytecode").unwrap();
        assert!(!second_paths
            .generated_upper
            .join(".venv/lib/private/__pycache__/module.pyc")
            .exists());
        assert!(first_paths
            .generated_upper
            .join(".venv/pyvenv.cfg")
            .is_file());
        assert!(second_paths
            .generated_upper
            .join(".venv/pyvenv.cfg")
            .is_file());
        assert!(python.path.is_absolute());
    }

    #[test]
    fn component_selector_resolves_nested_discovered_adapter_without_path() {
        if resolve_python_executable().is_err() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        let component_root = workspace.path().join("crates/python");
        fs::create_dir_all(&component_root).unwrap();
        fs::write(
            component_root.join("pyproject.toml"),
            "[project]\nname = \"nested-example\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "python-nested",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();

        let plan = db
            .plan_workspace_environment_component(
                "python-nested",
                "auto",
                None,
                Some("python-venv:crates/python"),
            )
            .unwrap();

        assert_eq!(plan.component_id, "python-venv:crates/python");
        assert_eq!(plan.adapter_identity, "trail/python-venv@1");
        assert_eq!(plan.mount_path, "crates/python/.venv");
    }

    #[test]
    fn python_venv_is_keyed_private_and_initialized_in_the_private_upper() {
        if resolve_python_executable().is_err() || resolve_workspace_tool_executable("uv").is_err()
        {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("pyproject.toml"),
            "[project]\nname = \"example\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(workspace.path().join("uv.lock"), "version = 1\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "python",
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

        let discovery = db.discover_workspace_environment("python", None).unwrap();
        let component = discovery
            .components
            .iter()
            .find(|component| component.adapter_identity == "trail/python-venv@1")
            .unwrap();
        assert_eq!(component.component_id, "python-venv");
        let plan = db
            .plan_workspace_environment("python", "trail/python-venv@1", None)
            .unwrap();
        assert_eq!(plan.commands.len(), 2);
        assert_eq!(plan.commands[0].phase, "mounted_initialization");
        assert_eq!(plan.commands[1].phase, "mounted_initialization");
        assert_eq!(plan.commands[1].program, "uv");
        assert!(plan.commands[1].args.iter().any(|arg| arg == "--frozen"));
        #[cfg(target_os = "macos")]
        assert_eq!(
            plan.commands[0].args,
            [
                "-m",
                "venv",
                "--without-pip",
                "--copies",
                "{trail-mounted-output:venv}"
            ]
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            plan.commands[0].args,
            ["-m", "venv", "--without-pip", "{trail-mounted-output:venv}"]
        );
        assert_eq!(plan.outputs[0].mount_path, ".venv");
        assert_eq!(
            plan.outputs[0].policy,
            EnvironmentOutputPolicy::WritablePrivate
        );
        assert_eq!(
            plan.inputs
                .iter()
                .map(|input| input.source_path.as_str())
                .collect::<Vec<_>>(),
            ["pyproject.toml", "uv.lock"]
        );
        #[cfg(target_os = "linux")]
        if std::env::var_os("TRAIL_RUN_FUSE_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        #[cfg(target_os = "macos")]
        if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        #[cfg(windows)]
        if std::env::var_os("TRAIL_RUN_DOKAN_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        let synchronized = db
            .sync_workspace_environment_component("python", "trail/python-venv@1", None, None)
            .unwrap();
        assert!(synchronized.layers.is_empty());
        let output = &synchronized.generation.components[0].outputs[0];
        assert_eq!(output.policy, EnvironmentOutputPolicy::WritablePrivate);
        assert!(output.layer_id.is_none());
        assert!(db.list_workspace_layers().unwrap().is_empty());
        assert!(db
            .workspace_view_paths_for_lane("python")
            .unwrap()
            .generated_upper
            .join(".venv")
            .is_dir());
        assert!(db
            .workspace_view_paths_for_lane("python")
            .unwrap()
            .generated_upper
            .join(".venv/pyvenv.cfg")
            .is_file());
    }

    #[test]
    fn mounted_python_initialization_crash_helper() {
        let Some(workspace) = std::env::var_os("TRAIL_TEST_MOUNTED_PYTHON_WORKSPACE") else {
            return;
        };
        let db = Trail::open(PathBuf::from(workspace)).unwrap();
        let _ = db.sync_workspace_environment_component(
            "python-crash",
            "trail/python-venv@1",
            None,
            None,
        );
        panic!("mounted Python crash helper passed its requested crash point");
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn killing_mounted_python_initialization_never_exposes_a_partial_generation() {
        #[cfg(target_os = "linux")]
        if std::env::var_os("TRAIL_RUN_FUSE_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        #[cfg(target_os = "macos")]
        if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        #[cfg(windows)]
        if std::env::var_os("TRAIL_RUN_DOKAN_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        if resolve_python_executable().is_err() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("pyproject.toml"),
            "[project]\nname = \"crash-venv\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "python-crash",
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
        let paths = db.workspace_view_paths_for_lane("python-crash").unwrap();
        drop(db);

        let ready = workspace.path().join("mounted-python-crash.ready");
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "db::lane::workspace_python::tests::mounted_python_initialization_crash_helper",
                "--nocapture",
            ])
            .env("RUST_TEST_THREADS", "1")
            .env(
                "TRAIL_TEST_CRASH_AT",
                "environment_after_mounted_initialization",
            )
            .env("TRAIL_TEST_CRASH_READY", &ready)
            .env("TRAIL_TEST_MOUNTED_PYTHON_WORKSPACE", workspace.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        wait_for_mounted_crash_handshake(
            &mut child,
            &ready,
            "environment_after_mounted_initialization",
        );
        child.kill().unwrap();
        let _ = child.wait().unwrap();

        let reopened = Trail::open(workspace.path()).unwrap();
        assert!(reopened
            .active_environment_generation("python-crash")
            .unwrap()
            .is_none());
        assert!(!paths.generated_upper.join(".venv").exists());
        let states = reopened.workspace_environment_rows("python-crash").unwrap();
        assert_eq!(states.len(), 1);
        assert_eq!(states[0].status, "failed");
        assert!(!fs::read_dir(workspace.path().join(".trail/cache/staging"))
            .unwrap()
            .filter_map(std::result::Result::ok)
            .any(|entry| entry
                .file_name()
                .to_string_lossy()
                .starts_with("mounted-environment-envsync_")));
    }

    #[cfg(any(target_os = "linux", target_os = "macos", windows))]
    #[test]
    fn sync_all_initializes_nested_python_components_at_final_lane_paths() {
        #[cfg(target_os = "linux")]
        if std::env::var_os("TRAIL_RUN_FUSE_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        #[cfg(target_os = "macos")]
        if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        #[cfg(windows)]
        if std::env::var_os("TRAIL_RUN_DOKAN_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        if resolve_python_executable().is_err() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        for component in ["services/api", "services/worker"] {
            fs::create_dir_all(workspace.path().join(component)).unwrap();
            fs::write(
                workspace.path().join(component).join("pyproject.toml"),
                format!(
                    "[project]\nname = \"{}\"\nversion = \"0.1.0\"\n",
                    component.replace('/', "-")
                ),
            )
            .unwrap();
        }
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "python-all",
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
        let report = db
            .sync_all_workspace_environments("python-all", None)
            .unwrap();
        assert!(report.layers.is_empty());
        assert_eq!(report.generation.components.len(), 2);

        #[cfg(target_os = "macos")]
        let mounted = db.mount_nfs_cow_workdir_for_lane("python-all").unwrap();
        #[cfg(any(target_os = "linux", windows))]
        let mounted = db.mount_fuse_cow_workdir_for_lane("python-all").unwrap();
        let workdir = PathBuf::from(db.lane_workdir("python-all").unwrap().workdir.unwrap());
        let paths = db.workspace_view_paths_for_lane("python-all").unwrap();
        for component in ["services/api", "services/worker"] {
            let venv = workdir.join(component).join(".venv");
            let direct_venv = paths.generated_upper.join(component).join(".venv");
            assert!(venv.join("pyvenv.cfg").is_file());
            #[cfg(windows)]
            let executable = venv.join("Scripts/python.exe");
            #[cfg(not(windows))]
            let executable = venv.join("bin/python");
            let prefix = Command::new(executable)
                .args(["-c", "import sys; print(sys.prefix)"])
                .output()
                .unwrap();
            assert!(prefix.status.success());
            #[cfg(windows)]
            assert_eq!(
                fs::canonicalize(String::from_utf8(prefix.stdout).unwrap().trim()).unwrap(),
                fs::canonicalize(&direct_venv).unwrap()
            );
            #[cfg(not(windows))]
            assert_eq!(
                String::from_utf8(prefix.stdout).unwrap().trim(),
                direct_venv.to_string_lossy()
            );
        }
        drop(mounted);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn real_python_venvs_use_direct_private_bindings_and_remain_isolated() {
        #[cfg(target_os = "linux")]
        if std::env::var_os("TRAIL_RUN_FUSE_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        #[cfg(target_os = "macos")]
        if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        if resolve_python_executable().is_err() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("pyproject.toml"),
            "[project]\nname = \"real-venv\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        for lane in ["python-a", "python-b"] {
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
            let report = db
                .sync_workspace_environment_component(lane, "trail/python-venv@1", None, None)
                .unwrap();
            assert!(report.layers.is_empty());

            #[cfg(target_os = "macos")]
            let mounted = db.mount_nfs_cow_workdir_for_lane(lane).unwrap();
            #[cfg(target_os = "linux")]
            let mounted = db.mount_fuse_cow_workdir_for_lane(lane).unwrap();
            let workdir = PathBuf::from(db.lane_workdir(lane).unwrap().workdir.unwrap());
            assert!(workdir.join(".venv/pyvenv.cfg").is_file());
            let environment = db
                .lane_workspace_environment(lane)
                .unwrap()
                .into_iter()
                .collect::<BTreeMap<_, _>>();
            let paths = db.workspace_view_paths_for_lane(lane).unwrap();
            let direct_venv = paths.generated_upper.join(".venv");
            assert_eq!(Path::new(&environment["VIRTUAL_ENV"]), direct_venv);
            assert_eq!(
                Path::new(&environment["TRAIL_VENV_PYTHON"]),
                direct_venv.join("bin/python")
            );
            let venv_python = direct_venv.join("bin/python");
            let prefix = Command::new(&venv_python)
                .args(["-c", "import sys; print(sys.prefix)"])
                .current_dir(&workdir)
                .output()
                .unwrap();
            assert!(prefix.status.success());
            assert_eq!(
                String::from_utf8(prefix.stdout).unwrap().trim(),
                direct_venv.to_string_lossy()
            );
            if lane == "python-a" {
                fs::write(workdir.join(".venv/lane-a.txt"), "private\n").unwrap();
            } else {
                assert!(!workdir.join(".venv/lane-a.txt").exists());
            }
            drop(mounted);
        }

        let unchanged = db
            .sync_workspace_environment_component("python-a", "trail/python-venv@1", None, None)
            .unwrap();
        assert!(unchanged.layers.is_empty());
        #[cfg(target_os = "macos")]
        let mounted = db.mount_nfs_cow_workdir_for_lane("python-a").unwrap();
        #[cfg(target_os = "linux")]
        let mounted = db.mount_fuse_cow_workdir_for_lane("python-a").unwrap();
        let workdir = PathBuf::from(db.lane_workdir("python-a").unwrap().workdir.unwrap());
        assert!(workdir.join(".venv/lane-a.txt").is_file());
        drop(mounted);
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn real_windows_python_venvs_embed_lane_paths_and_remain_isolated() {
        if std::env::var_os("TRAIL_RUN_DOKAN_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        if resolve_python_executable().is_err() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("pyproject.toml"),
            "[project]\nname = \"real-venv\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        for lane in ["python-a", "python-b"] {
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                lane,
                Some("main"),
                LaneWorkdirMode::FuseCow,
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
            let report = db
                .sync_workspace_environment_component(lane, "trail/python-venv@1", None, None)
                .unwrap();
            assert!(report.layers.is_empty());

            let mounted = db.mount_fuse_cow_workdir_for_lane(lane).unwrap();
            let workdir = PathBuf::from(db.lane_workdir(lane).unwrap().workdir.unwrap());
            assert!(workdir.join(".venv/pyvenv.cfg").is_file());
            let venv_python = workdir.join(".venv/Scripts/python.exe");
            let prefix = Command::new(&venv_python)
                .args(["-c", "import sys; print(sys.prefix)"])
                .current_dir(&workdir)
                .output()
                .unwrap();
            assert!(prefix.status.success());
            let actual_prefix =
                fs::canonicalize(String::from_utf8(prefix.stdout).unwrap().trim()).unwrap();
            let expected_prefix = fs::canonicalize(
                db.workspace_view_paths_for_lane(lane)
                    .unwrap()
                    .generated_upper
                    .join(".venv"),
            )
            .unwrap();
            assert_eq!(actual_prefix, expected_prefix);
            if lane == "python-a" {
                fs::write(workdir.join(".venv/lane-a.txt"), "private\n").unwrap();
            } else {
                assert!(!workdir.join(".venv/lane-a.txt").exists());
            }
            drop(mounted);
        }

        let unchanged = db
            .sync_workspace_environment_component("python-a", "trail/python-venv@1", None, None)
            .unwrap();
        assert!(unchanged.layers.is_empty());
        let mounted = db.mount_fuse_cow_workdir_for_lane("python-a").unwrap();
        let workdir = PathBuf::from(db.lane_workdir("python-a").unwrap().workdir.unwrap());
        assert!(workdir.join(".venv/lane-a.txt").is_file());
        drop(mounted);
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }
}
