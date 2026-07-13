use super::workspace_environment::{
    resolve_workspace_tool_executable, WorkspaceEnvironmentAdapter,
    WorkspaceEnvironmentAdapterMetadata, WorkspaceEnvironmentCacheAccess,
    WorkspaceEnvironmentCacheProtocol, WorkspaceEnvironmentCommand, WorkspaceEnvironmentInput,
    WorkspaceEnvironmentOutput, WorkspaceEnvironmentOutputPolicy, WorkspaceEnvironmentPlan,
    WorkspaceEnvironmentSandboxPolicy,
};
use super::*;

pub(crate) struct NodeWorkspaceAdapter;

pub(crate) static NODE_WORKSPACE_ADAPTER: NodeWorkspaceAdapter = NodeWorkspaceAdapter;

static NODE_WORKSPACE_ADAPTER_METADATA: WorkspaceEnvironmentAdapterMetadata =
    WorkspaceEnvironmentAdapterMetadata {
        canonical_identity: "trail/node@1",
        namespace: "trail",
        name: "node",
        contract_major: 1,
        implementation_version: env!("CARGO_PKG_VERSION"),
        distribution_digest: "builtin:node-plan-v1",
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

    fn detect(&self, db: &Trail, source_root: &ObjectId, component_root: &str) -> Result<bool> {
        let root = normalize_package_root(component_root)?;
        if db
            .root_file_entry(source_root, &join_repo_path(&root, "package.json"))?
            .is_none()
        {
            return Ok(false);
        }
        for (name, _) in supported_lockfiles() {
            if db
                .root_file_entry(source_root, &join_repo_path(&root, name))?
                .is_some()
            {
                return Ok(true);
            }
        }
        Ok(false)
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
        let mut selected = None;
        for (name, manager) in supported_lockfiles() {
            let path = join_repo_path(&package_root, name);
            if let Some(entry) = self.root_file_entry(root_id, &path)? {
                selected = Some((path, manager.to_string(), entry));
                break;
            }
        }
        let (lock_path, manager, lock_entry) = selected.ok_or_else(|| {
            Error::InvalidInput(format!(
                "Node package root `{}` has no supported frozen-install lockfile",
                if package_root.is_empty() {
                    "."
                } else {
                    &package_root
                }
            ))
        })?;
        let manager_version = tool_version(&manager)?;
        let node_version = tool_version("node")?;
        let node_tool = resolve_workspace_tool_executable("node")?;
        let manager_tool = resolve_workspace_tool_executable(&manager)?;
        let package_projection = self.project_entry_file(&package_entry)?;
        let package_text = fs::read_to_string(package_projection)?;
        let package_value: serde_json::Value = serde_json::from_str(&package_text)?;
        if package_value.get("workspaces").is_some() {
            return Err(Error::InvalidInput(format!(
                "Node component `{}` declares workspaces; synchronize a supported leaf package explicitly until the monorepo adapter is enabled",
                display_package_root(&package_root)
            )));
        }
        if contains_local_node_dependency(&package_value) {
            return Err(Error::InvalidInput(format!(
                "Node component `{}` contains file:, link:, or workspace: dependencies that cannot be represented by an isolated node_modules layer",
                display_package_root(&package_root)
            )));
        }
        if manager == "pnpm"
            && self
                .root_file_entry(
                    root_id,
                    &join_repo_path(&package_root, "pnpm-workspace.yaml"),
                )?
                .is_some()
        {
            return Err(Error::InvalidInput(
                "pnpm workspace roots require the future monorepo environment adapter".to_string(),
            ));
        }
        if manager == "yarn"
            && (!manager_version.starts_with('1')
                || self
                    .root_file_entry(root_id, &join_repo_path(&package_root, ".yarnrc.yml"))?
                    .is_some())
        {
            return Err(Error::InvalidInput(
                "Yarn Berry/PnP layouts are not node_modules layers; use Yarn Classic or wait for the PnP adapter"
                    .to_string(),
            ));
        }
        let mut files = BTreeMap::from([
            (package_json.clone(), package_entry.clone()),
            (lock_path.clone(), lock_entry.clone()),
        ]);
        for name in [
            ".npmrc",
            ".yarnrc",
            "pnpmfile.cjs",
            ".node-version",
            ".nvmrc",
        ] {
            let path = join_repo_path(&package_root, name);
            if let Some(entry) = self.root_file_entry(root_id, &path)? {
                files.insert(path, entry);
            }
        }
        let implementation_version = env!("CARGO_PKG_VERSION").to_string();
        let distribution_digest = "builtin:node-plan-v1".to_string();
        let mut key_inputs = files
            .iter()
            .map(|(path, entry)| (path.clone(), entry.content_hash.clone()))
            .collect::<BTreeMap<_, _>>();
        key_inputs.insert(
            "adapter_implementation".to_string(),
            implementation_version.clone(),
        );
        key_inputs.insert(
            "adapter_distribution_digest".to_string(),
            distribution_digest.clone(),
        );
        let key = WorkspaceLayerKeyV1 {
            kind: "dependency".to_string(),
            adapter: "node".to_string(),
            adapter_version: 1,
            inputs: key_inputs,
            tool_versions: BTreeMap::from([
                ("node".to_string(), node_version),
                (manager.clone(), manager_version),
                ("node-executable".to_string(), node_tool.identity),
                (
                    format!("{manager}-executable"),
                    manager_tool.identity.clone(),
                ),
            ]),
            platform: std::env::consts::OS.to_string(),
            architecture: std::env::consts::ARCH.to_string(),
            portability_scope: "platform-architecture-node-abi".to_string(),
            strategy: format!("{manager}-frozen-ignore-scripts-v1"),
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
        let environment = BTreeMap::from([
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
        ]);
        let args = match manager.as_str() {
            "npm" => vec!["ci", "--ignore-scripts", "--no-audit", "--no-fund"],
            "pnpm" => vec!["install", "--frozen-lockfile", "--ignore-scripts"],
            "yarn" => vec!["install", "--frozen-lockfile", "--ignore-scripts"],
            "bun" => vec!["install", "--frozen-lockfile", "--ignore-scripts"],
            other => {
                return Err(Error::InvalidInput(format!(
                    "unsupported Node package manager `{other}`"
                )));
            }
        }
        .into_iter()
        .map(str::to_string)
        .collect();
        let inputs = files
            .into_iter()
            .map(|(source_path, entry)| {
                let relative = strip_package_root(&source_path, &package_root)?;
                Ok(WorkspaceEnvironmentInput {
                    source_path,
                    staging_path: format!("project/{relative}"),
                    entry,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let mount_path = if package_root.is_empty() {
            "node_modules".to_string()
        } else {
            format!("{package_root}/node_modules")
        };
        Ok(WorkspaceEnvironmentPlan {
            component_id: NODE_WORKSPACE_ADAPTER.component_id(&package_root)?,
            adapter_identity: NODE_WORKSPACE_ADAPTER.identity().to_string(),
            adapter_version: 1,
            implementation_version,
            distribution_digest,
            kind: "dependency".to_string(),
            dependencies: Vec::new(),
            resolved_dependencies: Vec::new(),
            layer_key: key,
            inputs,
            source_projection: None,
            pre_commands: Vec::new(),
            command: Some(WorkspaceEnvironmentCommand {
                program: manager,
                resolved_program: manager_tool.path,
                executable_identity: manager_tool.identity,
                args,
                working_directory: project.clone(),
                environment,
                remove_environment: Vec::new(),
                cache_names: vec![cache.name.clone()],
            }),
            mounted_commands: Vec::new(),
            caches: vec![cache],
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
            outputs: vec![WorkspaceEnvironmentOutput {
                name: "modules".to_string(),
                output_path: format!("{project}/node_modules"),
                mount_path,
                policy: WorkspaceEnvironmentOutputPolicy::ImmutableSeedPrivate,
                create_if_missing: true,
            }],
            stale_reason:
                "package, lockfile, Node runtime, package manager, or adapter policy changed"
                    .to_string(),
        })
    }
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

    #[test]
    fn two_lanes_with_identical_node_inputs_reuse_one_real_frozen_install() {
        if Command::new("npm").arg("--version").output().is_err()
            || Command::new("node").arg("--version").output().is_err()
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
        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        for lane in ["node-one", "node-two"] {
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
