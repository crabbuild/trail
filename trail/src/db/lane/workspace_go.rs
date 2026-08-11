use super::workspace_environment::{
    resolve_workspace_tool_executable, WorkspaceEnvironmentAdapter,
    WorkspaceEnvironmentAdapterMetadata, WorkspaceEnvironmentCacheAccess,
    WorkspaceEnvironmentCacheCommandBinding, WorkspaceEnvironmentCacheProtocol,
    WorkspaceEnvironmentCommand, WorkspaceEnvironmentCommandBinding,
    WorkspaceEnvironmentConstructionSeed, WorkspaceEnvironmentOutput,
    WorkspaceEnvironmentOutputPolicy, WorkspaceEnvironmentPlan, WorkspaceEnvironmentSandboxPolicy,
    WorkspaceEnvironmentToolCommandBinding,
};
use super::*;

pub(crate) struct GoVendorAdapter;

pub(crate) static GO_VENDOR_ADAPTER: GoVendorAdapter = GoVendorAdapter;

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
        if db
            .root_file_entry(source_root, &join_repo_path(&component_root, "go.work"))?
            .is_some()
        {
            return Err(Error::InvalidInput(
                "Go workspaces require a multi-module graph adapter; trail/go-vendor@1 supports one module root"
                    .to_string(),
            ));
        }
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
