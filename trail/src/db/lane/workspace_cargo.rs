use super::workspace_environment::{
    resolve_workspace_tool_executable, WorkspaceEnvironmentAdapter,
    WorkspaceEnvironmentAdapterMetadata, WorkspaceEnvironmentCacheAccess,
    WorkspaceEnvironmentCacheProtocol, WorkspaceEnvironmentCommand, WorkspaceEnvironmentOutput,
    WorkspaceEnvironmentOutputPolicy, WorkspaceEnvironmentPlan, WorkspaceEnvironmentSandboxPolicy,
};
use super::*;

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
            .is_some()
            && db
                .root_file_entry(source_root, &join_repo_path(&root, "Cargo.lock"))?
                .is_some())
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
        let lock = db.root_file_entry(source_root, &lock_path)?.ok_or_else(|| {
            Error::InvalidInput(format!(
                "Cargo component `{}` has no Cargo.lock; generate and record a lockfile before synchronizing a target seed",
                display_component_root(&component_root)
            ))
        })?;
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
            component_id: self.component_id(&component_root)?,
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
                    (lock_path, lock.content_hash),
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
                create_if_missing: false,
            }],
            stale_reason:
                "source root, Cargo lockfile, Rust toolchain, target, or build policy changed"
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
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else {
            LaneWorkdirMode::OverlayCow
        };
        for lane in ["cargo-one", "cargo-two"] {
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
