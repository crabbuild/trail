use super::workspace_environment::{
    resolve_workspace_tool_executable, WorkspaceEnvironmentAdapter,
    WorkspaceEnvironmentAdapterMetadata, WorkspaceEnvironmentAdapterProposal,
    WorkspaceEnvironmentCacheAccess, WorkspaceEnvironmentCacheProtocol,
    WorkspaceEnvironmentCommand, WorkspaceEnvironmentOutput, WorkspaceEnvironmentOutputPolicy,
    WorkspaceEnvironmentPlan, WorkspaceEnvironmentResolutionInput,
    WorkspaceEnvironmentSandboxPolicy,
};
use super::*;
use crate::ids::sha256_hex;

pub(crate) struct CargoTargetSeedAdapter;

pub(crate) static CARGO_TARGET_SEED_ADAPTER: CargoTargetSeedAdapter = CargoTargetSeedAdapter;

static CARGO_TARGET_SEED_ADAPTER_METADATA: WorkspaceEnvironmentAdapterMetadata =
    WorkspaceEnvironmentAdapterMetadata {
        canonical_identity: "trail/cargo-target-seed@1",
        namespace: "trail",
        name: "cargo-target-seed",
        contract_major: 1,
        implementation_version: env!("CARGO_PKG_VERSION"),
        distribution_digest: "builtin:cargo-target-seed-plan-v1",
        selectors: &["trail/cargo-target-seed@1", "cargo-target-seed", "cargo"],
        kind: "compiler-results",
        layer_adapter_name: "cargo-target-seed",
        discovery_markers: &["Cargo.toml"],
        supported_operating_systems: &["linux", "macos", "windows"],
        supported_architectures: &["aarch64", "x86_64"],
        stability: "experimental",
        description:
            "Locked Cargo target seed keyed by the complete source root and Rust toolchain identity",
    };

impl WorkspaceEnvironmentAdapter for CargoTargetSeedAdapter {
    fn metadata(&self) -> &'static WorkspaceEnvironmentAdapterMetadata {
        &CARGO_TARGET_SEED_ADAPTER_METADATA
    }

    fn component_id(&self, component_root: &str) -> Result<String> {
        let root = normalize_component_root(component_root)?;
        Ok(if root.is_empty() {
            "cargo-target-seed".to_string()
        } else {
            format!("cargo-target-seed:{root}")
        })
    }

    fn detect(&self, db: &Trail, source_root: &ObjectId, component_root: &str) -> Result<bool> {
        let root = normalize_component_root(component_root)?;
        Ok(db
            .root_file_entry(source_root, &join_repo_path(&root, "Cargo.toml"))?
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
            .root_file_entry(source_root, &join_repo_path(&root, "Cargo.toml"))?
            .is_none()
        {
            return Ok(None);
        }
        if db
            .root_file_entry(source_root, &join_repo_path(&root, "Cargo.lock"))?
            .is_some()
        {
            return Ok(Some(WorkspaceEnvironmentAdapterProposal::ready()));
        }
        if cargo_resolution_snapshot(db, source_root, &root)?.is_some() {
            return Ok(Some(WorkspaceEnvironmentAdapterProposal::ready()));
        }
        Ok(Some(WorkspaceEnvironmentAdapterProposal::resolvable(
            EnvironmentProposalReasonReport {
                code: "resolution_snapshot_missing".to_string(),
                message: "Cargo.toml is present but no Cargo.lock or Trail-managed resolution snapshot is available".to_string(),
            },
            EnvironmentRecoveryActionReport {
                code: "resolve_component".to_string(),
                description: "Resolve and pin a Trail-managed Cargo.lock snapshot without adding it to source".to_string(),
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
        let root = normalize_component_root(component_root)?;
        let manifest_path = join_repo_path(&root, "Cargo.toml");
        let Some(manifest) = db.root_file_entry(source_root, &manifest_path)? else {
            return Ok(None);
        };
        let cargo_tool = resolve_workspace_tool_executable("cargo")?;
        let proposal_key = cargo_resolution_proposal_key(source_root, &root);
        let policy_identity = sha256_hex(
            format!(
                "cargo-lock-resolver-v1\0{}\0offline\0scripts-denied",
                cargo_tool.identity
            )
            .as_bytes(),
        );
        Ok(Some(ArtifactResolutionPlanV1 {
            version: ARTIFACT_RESOLUTION_PLAN_VERSION,
            proposal_key,
            source_root: source_root.clone(),
            component_id: self.component_id(&root)?,
            adapter_identity: self.identity().to_string(),
            policy_identity,
            program: "cargo".to_string(),
            resolved_program: cargo_tool.path.to_string_lossy().into_owned(),
            executable_identity: cargo_tool.identity,
            argv: vec![
                "cargo".to_string(),
                "generate-lockfile".to_string(),
                "--offline".to_string(),
            ],
            working_directory: if root.is_empty() {
                ".".to_string()
            } else {
                root.clone()
            },
            readable_inputs: vec![ArtifactResolutionInputV1 {
                source_path: manifest_path,
                content_hash: manifest.content_hash,
                size_bytes: manifest.size_bytes,
            }],
            candidate_output: join_repo_path(&root, "Cargo.lock"),
            allowed_authorities: Vec::new(),
            credential_handles: Vec::new(),
            script_policy: ArtifactScriptPolicyV1::Deny,
            environment_roles: BTreeMap::new(),
            limits: ArtifactActionLimitsV1 {
                timeout_ms: 5 * 60 * 1_000,
                stdout_bytes: 1024 * 1024,
                stderr_bytes: 1024 * 1024,
                candidate_bytes: 16 * 1024 * 1024,
                candidate_entries: 1,
                child_processes: 32,
            },
            snapshot_format: "cargo-lock-toml-v1".to_string(),
            validations: vec![ArtifactValidationV1 {
                name: "cargo-lock-structure".to_string(),
                kind: ArtifactValidationKindV1::Framework,
                required: true,
                parameters: BTreeMap::from([("format".to_string(), "toml".to_string())]),
            }],
        }))
    }

    fn plan(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<WorkspaceEnvironmentPlan> {
        let component_root = normalize_component_root(component_root)?;
        let manifest_path = join_repo_path(&component_root, "Cargo.toml");
        let lock_path = join_repo_path(&component_root, "Cargo.lock");
        let manifest = db
            .root_file_entry(source_root, &manifest_path)?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "Cargo component `{}` has no Cargo.toml",
                    display_component_root(&component_root)
                ))
            })?;
        let source_lock = db.root_file_entry(source_root, &lock_path)?;
        let component_id = self.component_id(&component_root)?;
        let (lock_content_hash, lock_authority, resolution_inputs) = if let Some(lock) = source_lock
        {
            (lock.content_hash, "source".to_string(), Vec::new())
        } else {
            let resolution_plan = self
                .resolution_plan(db, source_root, &component_root)?
                .ok_or_else(|| {
                    Error::Corrupt(format!(
                        "Cargo component `{}` lost its resolver plan",
                        display_component_root(&component_root)
                    ))
                })?;
            let (snapshot_id, snapshot, bytes) =
                    cargo_resolution_snapshot(db, source_root, &component_root)?.ok_or_else(|| {
                        Error::InvalidInput(format!(
                            "Cargo component `{}` has no Cargo.lock or Trail-managed resolution snapshot; resolve it through Trail's artifact resolution operation before synchronizing",
                            display_component_root(&component_root)
                        ))
                    })?;
            if snapshot.resolver_executable_identity != resolution_plan.executable_identity
                || snapshot.policy_identity != resolution_plan.policy_identity
            {
                return Err(Error::InvalidInput(format!(
                    "Cargo component `{}` resolution snapshot was created by a different Cargo executable or resolver policy; resolve it again for the current toolchain",
                    display_component_root(&component_root)
                )));
            }
            validate_cargo_lock_snapshot(&bytes)?;
            let size_bytes = u64::try_from(bytes.len()).map_err(|_| {
                Error::InvalidInput("Cargo lock snapshot exceeds platform limits".to_string())
            })?;
            (
                snapshot.content_sha256.clone(),
                format!("snapshot:{}", snapshot_id.0),
                vec![WorkspaceEnvironmentResolutionInput {
                    snapshot_id,
                    source_root: source_root.clone(),
                    source_path: lock_path.clone(),
                    staging_path: format!("project/{lock_path}"),
                    content_hash: snapshot.content_sha256,
                    size_bytes,
                }],
            )
        };
        let cargo_version = command_identity("cargo", &["--version"])?;
        let rustc_identity = command_identity("rustc", &["-vV"])?;
        let cargo_tool = resolve_workspace_tool_executable("cargo")?;
        let rustc_tool = resolve_workspace_tool_executable("rustc")?;
        let host_target = rustc_identity
            .lines()
            .find_map(|line| line.strip_prefix("host: "))
            .unwrap_or("unknown")
            .to_string();
        let has_sccache = command_is_available("sccache");
        let mut tool_versions = BTreeMap::from([
            ("cargo".to_string(), cargo_version.clone()),
            ("rustc-vV".to_string(), rustc_identity.clone()),
            ("target".to_string(), host_target.clone()),
            ("cargo-executable".to_string(), cargo_tool.identity.clone()),
            ("rustc-executable".to_string(), rustc_tool.identity),
        ]);
        let cargo_cache = db.declare_workspace_environment_cache(
            self.identity(),
            "cargo-home",
            WorkspaceEnvironmentCacheProtocol::LockedIndex,
            WorkspaceEnvironmentCacheAccess::ToolConcurrent,
            BTreeMap::from([
                ("cargo".to_string(), cargo_version),
                ("cargo_executable".to_string(), cargo_tool.identity.clone()),
                ("platform".to_string(), std::env::consts::OS.to_string()),
                (
                    "architecture".to_string(),
                    std::env::consts::ARCH.to_string(),
                ),
            ]),
        )?;
        let mut caches = vec![cargo_cache.clone()];
        let mut cache_names = vec![cargo_cache.name.clone()];
        let mut environment = BTreeMap::from([
            (
                "CARGO_HOME".to_string(),
                cargo_cache.storage_path.to_string_lossy().into_owned(),
            ),
            ("CARGO_NET_OFFLINE".to_string(), "true".to_string()),
            (
                "CARGO_INCREMENTAL".to_string(),
                if has_sccache { "0" } else { "1" }.to_string(),
            ),
        ]);
        let rustup_home = std::env::var_os("RUSTUP_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".rustup")));
        if let Some(rustup_home) = rustup_home.filter(|path| path.is_dir()) {
            environment.insert(
                "RUSTUP_HOME".to_string(),
                rustup_home.to_string_lossy().into_owned(),
            );
        }
        let rustup_toolchain = std::env::var("RUSTUP_TOOLCHAIN").ok();
        if let Some(toolchain) = &rustup_toolchain {
            environment.insert("RUSTUP_TOOLCHAIN".to_string(), toolchain.clone());
        }
        if has_sccache {
            let sccache_tool = resolve_workspace_tool_executable("sccache")?;
            let sccache_version = command_identity("sccache", &["--version"])?;
            let sccache_cache = db.declare_workspace_environment_cache(
                self.identity(),
                "sccache",
                WorkspaceEnvironmentCacheProtocol::CompilerCache,
                WorkspaceEnvironmentCacheAccess::ToolConcurrent,
                BTreeMap::from([
                    ("sccache".to_string(), sccache_version.clone()),
                    ("rustc".to_string(), rustc_identity),
                    ("target".to_string(), host_target),
                    ("platform".to_string(), std::env::consts::OS.to_string()),
                    (
                        "architecture".to_string(),
                        std::env::consts::ARCH.to_string(),
                    ),
                ]),
            )?;
            environment.insert(
                "RUSTC_WRAPPER".to_string(),
                sccache_tool.path.to_string_lossy().into_owned(),
            );
            environment.insert(
                "SCCACHE_DIR".to_string(),
                sccache_cache.storage_path.to_string_lossy().into_owned(),
            );
            // A long-lived sccache server can outlive an attempt-owned temp
            // directory. Cache I/O loss must degrade to rustc, not fail the
            // deterministic target-seed build.
            environment.insert(
                "SCCACHE_IGNORE_SERVER_IO_ERROR".to_string(),
                "1".to_string(),
            );
            tool_versions.insert("sccache".to_string(), sccache_version);
            tool_versions.insert("sccache-executable".to_string(), sccache_tool.identity);
            cache_names.push(sccache_cache.name.clone());
            caches.push(sccache_cache);
        }
        let mut remove_environment = vec![
            "CARGO_TARGET_DIR".to_string(),
            "CARGO_ENCODED_RUSTFLAGS".to_string(),
            "RUSTFLAGS".to_string(),
            "RUSTDOCFLAGS".to_string(),
            "RUSTC_WORKSPACE_WRAPPER".to_string(),
        ];
        if !has_sccache {
            remove_environment.push("RUSTC_WRAPPER".to_string());
        }
        let mut fetch_environment = environment.clone();
        fetch_environment.insert("CARGO_NET_OFFLINE".to_string(), "false".to_string());
        let working_directory = if component_root.is_empty() {
            "project".to_string()
        } else {
            format!("project/{component_root}")
        };
        let output_path = format!("{working_directory}/target");
        let mount_path = if component_root.is_empty() {
            "target".to_string()
        } else {
            format!("{component_root}/target")
        };
        Ok(WorkspaceEnvironmentPlan {
            component_id,
            adapter_identity: self.identity().to_string(),
            adapter_version: 1,
            implementation_version: env!("CARGO_PKG_VERSION").to_string(),
            distribution_digest: "builtin:cargo-target-seed-plan-v1".to_string(),
            kind: "compiler-results".to_string(),
            dependencies: Vec::new(),
            resolved_dependencies: Vec::new(),
            layer_key: WorkspaceLayerKeyV1 {
                kind: "compiler-results".to_string(),
                adapter: self.layer_adapter_name().to_string(),
                adapter_version: 1,
                inputs: BTreeMap::from([
                    ("source_root".to_string(), source_root.0.clone()),
                    (manifest_path, manifest.content_hash),
                    (lock_path, lock_content_hash),
                    ("lock_authority".to_string(), lock_authority),
                    (
                        "output_contract".to_string(),
                        format!("immutable-seed-private:{mount_path}"),
                    ),
                    (
                        "adapter_implementation".to_string(),
                        env!("CARGO_PKG_VERSION").to_string(),
                    ),
                    (
                        "adapter_distribution_digest".to_string(),
                        "builtin:cargo-target-seed-plan-v1".to_string(),
                    ),
                    (
                        "rustup_toolchain".to_string(),
                        rustup_toolchain.unwrap_or_default(),
                    ),
                ]),
                tool_versions,
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "source-root-toolchain-target-platform".to_string(),
                strategy: format!(
                    "cargo-build-locked-offline-target-seed-v1:{}",
                    if has_sccache {
                        "sccache"
                    } else {
                        "incremental"
                    }
                ),
            },
            inputs: Vec::new(),
            resolution_inputs,
            source_projection: Some((source_root.clone(), "project".to_string())),
            pre_commands: vec![WorkspaceEnvironmentCommand {
                program: "cargo".to_string(),
                resolved_program: cargo_tool.path.clone(),
                executable_identity: cargo_tool.identity.clone(),
                args: vec!["fetch".to_string(), "--locked".to_string()],
                working_directory: working_directory.clone(),
                environment: fetch_environment,
                remove_environment: remove_environment.clone(),
                cache_names: cache_names.clone(),
            }],
            command: Some(WorkspaceEnvironmentCommand {
                program: "cargo".to_string(),
                resolved_program: cargo_tool.path,
                executable_identity: cargo_tool.identity,
                args: vec![
                    "build".to_string(),
                    "--locked".to_string(),
                    "--offline".to_string(),
                    "--target-dir".to_string(),
                    "target".to_string(),
                ],
                working_directory,
                environment,
                remove_environment,
                cache_names,
            }),
            mounted_commands: Vec::new(),
            caches,
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
            outputs: vec![WorkspaceEnvironmentOutput {
                name: "target-seed".to_string(),
                output_path,
                mount_path,
                policy: WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate,
                reuse: EnvironmentReuseMode::Exact,
                scope: EnvironmentSharingScope::Workspace,
                publish: EnvironmentPublicationTrigger::OnSync,
                gate: None,
                create_if_missing: false,
            }],
            stale_reason:
                "source root, Cargo lockfile, Rust toolchain, target, or build policy changed"
                    .to_string(),
        })
    }
}

fn cargo_resolution_proposal_key(source_root: &ObjectId, component_root: &str) -> String {
    let identity = format!(
        "cargo-lock-resolution-v1\0{}\0{}\0{}",
        source_root.0, component_root, CARGO_TARGET_SEED_ADAPTER_METADATA.canonical_identity
    );
    format!("cargo_lock_v1_{}", sha256_hex(identity.as_bytes()))
}

fn cargo_resolution_snapshot(
    db: &Trail,
    source_root: &ObjectId,
    component_root: &str,
) -> Result<Option<(ObjectId, ArtifactResolutionSnapshotV1, Vec<u8>)>> {
    let proposal_key = cargo_resolution_proposal_key(source_root, component_root);
    let Some((snapshot_id, snapshot)) =
        db.artifact_resolution_snapshot_for_proposal(&proposal_key)?
    else {
        return Ok(None);
    };
    let expected_component = CARGO_TARGET_SEED_ADAPTER.component_id(component_root)?;
    if snapshot.source_root != *source_root
        || snapshot.component_id != expected_component
        || snapshot.adapter_identity != CARGO_TARGET_SEED_ADAPTER.identity()
        || snapshot.snapshot_format != "cargo-lock-toml-v1"
        || snapshot.verification_state != ArtifactResolutionVerificationStateV1::Verified
        || !snapshot.secret_taint.is_clear()
    {
        return Err(Error::Corrupt(format!(
            "Cargo resolution snapshot {snapshot_id} does not match proposal `{proposal_key}`"
        )));
    }
    let bytes = db.artifact_resolution_snapshot_content(&snapshot)?;
    validate_cargo_lock_snapshot(&bytes)?;
    Ok(Some((snapshot_id, snapshot, bytes)))
}

fn validate_cargo_lock_snapshot(bytes: &[u8]) -> Result<()> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        Error::InvalidInput("Trail-managed Cargo.lock snapshot is not UTF-8".to_string())
    })?;
    let document = toml::from_str::<toml::Value>(text).map_err(|error| {
        Error::InvalidInput(format!(
            "Trail-managed Cargo.lock snapshot is malformed TOML: {error}"
        ))
    })?;
    let version = document
        .get("version")
        .and_then(toml::Value::as_integer)
        .ok_or_else(|| {
            Error::InvalidInput(
                "Trail-managed Cargo.lock snapshot has no integer lockfile version".to_string(),
            )
        })?;
    if !(1..=4).contains(&version) {
        return Err(Error::InvalidInput(format!(
            "Trail-managed Cargo.lock snapshot version {version} is unsupported"
        )));
    }
    Ok(())
}

fn normalize_component_root(component_root: &str) -> Result<String> {
    if component_root.trim_matches('/').is_empty() {
        Ok(String::new())
    } else {
        normalize_relative_path(component_root)
    }
}

fn display_component_root(component_root: &str) -> &str {
    if component_root.is_empty() {
        "."
    } else {
        component_root
    }
}

fn join_repo_path(root: &str, name: &str) -> String {
    if root.is_empty() {
        name.to_string()
    } else {
        format!("{root}/{name}")
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

fn command_is_available(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
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
    fn manifest_only_cargo_uses_managed_lock_for_locked_offline_shared_seed() {
        if command_identity("cargo", &["--version"]).is_err()
            || command_identity("rustc", &["-vV"]).is_err()
        {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("src")).unwrap();
        fs::write(
            workspace.path().join("Cargo.toml"),
            "[package]\nname = \"trail-managed-lock-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("src/lib.rs"),
            "pub fn managed() -> bool { true }\n",
        )
        .unwrap();

        let resolver = tempfile::tempdir().unwrap();
        fs::create_dir_all(resolver.path().join("src")).unwrap();
        fs::copy(
            workspace.path().join("Cargo.toml"),
            resolver.path().join("Cargo.toml"),
        )
        .unwrap();
        fs::copy(
            workspace.path().join("src/lib.rs"),
            resolver.path().join("src/lib.rs"),
        )
        .unwrap();
        let generated = Command::new("cargo")
            .args(["generate-lockfile", "--offline"])
            .current_dir(resolver.path())
            .output()
            .unwrap();
        assert!(
            generated.status.success(),
            "cargo generate-lockfile failed: {}",
            String::from_utf8_lossy(&generated.stderr)
        );
        let lock_bytes = fs::read(resolver.path().join("Cargo.lock")).unwrap();

        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let source_root = db.get_ref("refs/branches/main").unwrap().root_id;
        assert!(CARGO_TARGET_SEED_ADAPTER
            .detect(&db, &source_root, "")
            .unwrap());
        let proposal = CARGO_TARGET_SEED_ADAPTER
            .propose(&db, &source_root, "")
            .unwrap()
            .unwrap();
        assert_eq!(
            proposal.status,
            EnvironmentComponentProposalStatus::Resolvable
        );
        let resolution_plan = CARGO_TARGET_SEED_ADAPTER
            .resolution_plan(&db, &source_root, "")
            .unwrap()
            .unwrap();
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
        assert_eq!(
            CARGO_TARGET_SEED_ADAPTER
                .propose(&db, &source_root, "")
                .unwrap()
                .unwrap()
                .status,
            EnvironmentComponentProposalStatus::Ready
        );

        for lane in ["managed-lock-one", "managed-lock-two"] {
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
            .sync_workspace_environment("managed-lock-one", "cargo", None)
            .unwrap();
        let second = db
            .sync_workspace_environment("managed-lock-two", "cargo", None)
            .unwrap();
        assert_eq!(first.layer_id, second.layer_id);
        assert_eq!(first.cache_key, second.cache_key);
        assert!(Path::new(&first.storage_path).join("debug").is_dir());
        assert!(!workspace.path().join("Cargo.lock").exists());
        assert!(db
            .root_file_entry(&source_root, "Cargo.lock")
            .unwrap()
            .is_none());

        fs::write(
            workspace.path().join("README.md"),
            "unrelated source change\n",
        )
        .unwrap();
        db.record(
            Some("main"),
            Some("change source root".to_string()),
            Actor::human(),
            false,
        )
        .unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "managed-lock-new-root",
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
            .discover_workspace_environment("managed-lock-new-root", None)
            .unwrap();
        assert_eq!(
            discovery.components[0].status,
            EnvironmentComponentProposalStatus::Resolvable
        );
        let error = db
            .sync_workspace_environment("managed-lock-new-root", "cargo", None)
            .unwrap_err();
        assert!(error
            .to_string()
            .contains("Trail-managed resolution snapshot"));
    }

    #[test]
    fn cargo_adapter_builds_once_and_reuses_one_immutable_target_seed() {
        if command_identity("cargo", &["--version"]).is_err()
            || command_identity("rustc", &["-vV"]).is_err()
        {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::create_dir_all(workspace.path().join("src")).unwrap();
        fs::write(
            workspace.path().join("Cargo.toml"),
            "[package]\nname = \"trail-cargo-adapter-test\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("src/lib.rs"),
            "pub fn answer() -> u64 { 42 }\n",
        )
        .unwrap();
        let lock = Command::new("cargo")
            .args(["generate-lockfile", "--offline"])
            .current_dir(workspace.path())
            .output()
            .unwrap();
        assert!(
            lock.status.success(),
            "cargo generate-lockfile failed: {}",
            String::from_utf8_lossy(&lock.stderr)
        );

        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        for lane in ["cargo-one", "cargo-two"] {
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
            .sync_workspace_environment("cargo-one", "auto", None)
            .unwrap();
        let second = db
            .sync_workspace_environment("cargo-two", "trail/cargo-target-seed@1", None)
            .unwrap();
        assert_eq!(first.layer_id, second.layer_id);
        assert_eq!(first.cache_key, second.cache_key);
        assert!(Path::new(&first.storage_path).join("debug").is_dir());

        let status = db.environment_component_status("cargo-two").unwrap();
        assert_eq!(status.len(), 1);
        assert_eq!(status[0].component.component_id, "cargo-target-seed");
        assert_eq!(status[0].adapter.name, "cargo-target-seed");
        assert_eq!(
            status[0].adapter.implementation_version,
            env!("CARGO_PKG_VERSION")
        );
        assert_eq!(
            status[0].adapter.distribution_digest.as_deref(),
            Some("builtin:cargo-target-seed-plan-v1")
        );
        assert_eq!(status[0].status, "ready");
    }
}
