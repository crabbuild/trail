use super::workspace_environment::{
    resolve_workspace_tool_executable, WorkspaceEnvironmentAdapter,
    WorkspaceEnvironmentAdapterMetadata, WorkspaceEnvironmentOutput,
    WorkspaceEnvironmentOutputPolicy, WorkspaceEnvironmentPlan, WorkspaceEnvironmentSandboxPolicy,
};
use super::*;

pub(crate) struct CmakeBuildTreeAdapter;

pub(crate) static CMAKE_BUILD_TREE_ADAPTER: CmakeBuildTreeAdapter = CmakeBuildTreeAdapter;

static CMAKE_BUILD_TREE_ADAPTER_METADATA: WorkspaceEnvironmentAdapterMetadata =
    WorkspaceEnvironmentAdapterMetadata {
        canonical_identity: "trail/cmake-build@1",
        namespace: "trail",
        name: "cmake-build",
        contract_major: 1,
        implementation_version: env!("CARGO_PKG_VERSION"),
        distribution_digest: "builtin:cmake-build-plan-v1",
        selectors: &["trail/cmake-build@1", "cmake-build", "cmake"],
        kind: "build",
        layer_adapter_name: "cmake-build",
        discovery_markers: &["CMakeLists.txt"],
        supported_operating_systems: &["linux", "macos", "windows"],
        supported_architectures: &["aarch64", "x86_64"],
        stability: "experimental",
        description: "Lane-private CMake build tree with configure deferred to the mounted lane",
    };

impl WorkspaceEnvironmentAdapter for CmakeBuildTreeAdapter {
    fn metadata(&self) -> &'static WorkspaceEnvironmentAdapterMetadata {
        &CMAKE_BUILD_TREE_ADAPTER_METADATA
    }

    fn component_id(&self, component_root: &str) -> Result<String> {
        let root = normalize_component_root(component_root)?;
        Ok(if root.is_empty() {
            "cmake-build".to_string()
        } else {
            format!("cmake-build:{root}")
        })
    }

    fn detect(&self, db: &Trail, source_root: &ObjectId, component_root: &str) -> Result<bool> {
        let root = normalize_component_root(component_root)?;
        Ok(db
            .root_file_entry(source_root, &join_repo_path(&root, "CMakeLists.txt"))?
            .is_some())
    }

    fn plan(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<WorkspaceEnvironmentPlan> {
        let component_root = normalize_component_root(component_root)?;
        let manifest_path = join_repo_path(&component_root, "CMakeLists.txt");
        if db.root_file_entry(source_root, &manifest_path)?.is_none() {
            return Err(Error::InvalidInput(format!(
                "CMake component `{}` has no CMakeLists.txt",
                display_component_root(&component_root)
            )));
        }
        let cmake = resolve_workspace_tool_executable("cmake")?;
        let implementation_version = env!("CARGO_PKG_VERSION").to_string();
        let distribution_digest = "builtin:cmake-build-plan-v1".to_string();
        let mount_path = join_repo_path(&component_root, "build");
        let component_id = self.component_id(&component_root)?;
        let inputs = BTreeMap::from([
            ("component_id".to_string(), component_id.clone()),
            ("component_root".to_string(), component_root.clone()),
            ("manifest".to_string(), manifest_path),
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
                "configure_phase".to_string(),
                "deferred-to-mounted-lane".to_string(),
            ),
        ]);
        Ok(WorkspaceEnvironmentPlan {
            component_id,
            adapter_identity: self.identity().to_string(),
            adapter_version: 1,
            implementation_version,
            distribution_digest,
            kind: "build".to_string(),
            dependencies: Vec::new(),
            resolved_dependencies: Vec::new(),
            layer_key: WorkspaceLayerKeyV1 {
                kind: "build".to_string(),
                adapter: self.layer_adapter_name().to_string(),
                adapter_version: 1,
                inputs,
                tool_versions: BTreeMap::from([("cmake-executable".to_string(), cmake.identity)]),
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "lane-private-host-tool".to_string(),
                strategy: "cmake-build-tree-private-v1".to_string(),
            },
            inputs: Vec::new(),
            source_projection: None,
            pre_commands: Vec::new(),
            command: None,
            mounted_commands: Vec::new(),
            caches: Vec::new(),
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
            outputs: vec![WorkspaceEnvironmentOutput {
                name: "build-tree".to_string(),
                // No command consumes this staging path. The host creates an
                // empty private directory directly in the final lane upper.
                output_path: "private/build".to_string(),
                mount_path,
                policy: WorkspaceEnvironmentOutputPolicy::WritablePrivate,
                create_if_missing: true,
            }],
            stale_reason:
                "CMake executable, host platform, architecture, component root, or adapter policy changed"
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
    use std::ffi::OsStr;

    #[test]
    fn cmake_discovery_is_pinned_and_side_effect_free() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3.20)\nproject(example)\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let root = db.resolve_branch_ref("main").unwrap().root_id;
        assert!(CMAKE_BUILD_TREE_ADAPTER.detect(&db, &root, "").unwrap());
        assert_eq!(
            CMAKE_BUILD_TREE_ADAPTER.component_id("").unwrap(),
            "cmake-build"
        );
        assert_eq!(
            CMAKE_BUILD_TREE_ADAPTER.component_id("native/lib").unwrap(),
            "cmake-build:native/lib"
        );
    }

    #[test]
    fn cmake_sync_provisions_private_state_without_publishing_a_layer() {
        if resolve_workspace_tool_executable("cmake").is_err() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3.20)\nproject(example)\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "cmake",
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
        let plan = db
            .plan_workspace_environment("cmake", "trail/cmake-build@1", None)
            .unwrap();
        assert!(plan.commands.is_empty());
        assert_eq!(plan.outputs[0].policy, "writable_private");
        assert!(plan.tools.contains_key("cmake-executable"));
        let report = db
            .sync_workspace_environment_component("cmake", "trail/cmake-build@1", None, None)
            .unwrap();
        assert!(report.layers.is_empty());
        assert_eq!(
            report.generation.components[0].outputs[0].policy,
            "writable_private"
        );
        assert!(report.generation.components[0].outputs[0]
            .layer_id
            .is_none());
    }

    #[cfg(unix)]
    #[test]
    fn real_cmake_configure_build_and_clean_stay_lane_private() {
        #[cfg(target_os = "linux")]
        if std::env::var_os("TRAIL_RUN_FUSE_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        #[cfg(target_os = "macos")]
        if std::env::var_os("TRAIL_RUN_NFS_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        if resolve_workspace_tool_executable("cmake").is_err()
            || resolve_workspace_tool_executable("make").is_err()
            || resolve_workspace_tool_executable("cc").is_err()
        {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3.20)\nproject(trail_lane C)\nadd_executable(hello main.c)\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("main.c"),
            "#include <stdio.h>\nint main(void) { puts(\"hello\"); return 0; }\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        for lane in ["cmake-a", "cmake-b"] {
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
                .sync_workspace_environment_component(lane, "trail/cmake-build@1", None, None)
                .unwrap();
            assert!(report.layers.is_empty());
        }

        for lane in ["cmake-a", "cmake-b"] {
            #[cfg(target_os = "macos")]
            let mounted = db.mount_nfs_cow_workdir_for_lane(lane).unwrap();
            #[cfg(target_os = "linux")]
            let mounted = db.mount_fuse_cow_workdir_for_lane(lane).unwrap();
            let workdir = PathBuf::from(db.lane_workdir(lane).unwrap().workdir.unwrap());
            let configured = Command::new("cmake")
                .args(["-S", ".", "-B", "build", "-G", "Unix Makefiles"])
                .current_dir(&workdir)
                .status()
                .unwrap();
            assert!(configured.success());
            let built = Command::new("cmake")
                .args(["--build", "build", "--parallel", "2"])
                .current_dir(&workdir)
                .status()
                .unwrap();
            assert!(built.success());
            assert!(workdir.join("build/hello").is_file());
            let cache = fs::read_to_string(workdir.join("build/CMakeCache.txt")).unwrap();
            assert!(cache.contains(workdir.to_string_lossy().as_ref()));
            drop(mounted);
        }

        #[cfg(target_os = "macos")]
        let mounted = db.mount_nfs_cow_workdir_for_lane("cmake-a").unwrap();
        #[cfg(target_os = "linux")]
        let mounted = db.mount_fuse_cow_workdir_for_lane("cmake-a").unwrap();
        let workdir_a = PathBuf::from(db.lane_workdir("cmake-a").unwrap().workdir.unwrap());
        let cleaned = Command::new("cmake")
            .args(["--build", "build", "--target", "clean"])
            .current_dir(&workdir_a)
            .status()
            .unwrap();
        assert!(cleaned.success());
        assert!(!workdir_a.join("build/hello").exists());
        drop(mounted);

        #[cfg(target_os = "macos")]
        let mounted = db.mount_nfs_cow_workdir_for_lane("cmake-b").unwrap();
        #[cfg(target_os = "linux")]
        let mounted = db.mount_fuse_cow_workdir_for_lane("cmake-b").unwrap();
        let workdir_b = PathBuf::from(db.lane_workdir("cmake-b").unwrap().workdir.unwrap());
        assert!(workdir_b.join("build/hello").is_file());
        drop(mounted);
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }

    #[cfg(windows)]
    #[test]
    fn real_windows_cmake_build_and_clean_stay_lane_private() {
        if std::env::var_os("TRAIL_RUN_DOKAN_COW_TESTS").as_deref() != Some(OsStr::new("1")) {
            return;
        }
        if resolve_workspace_tool_executable("cmake").is_err() {
            return;
        }
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3.20)\nproject(trail_lane C)\nadd_executable(hello main.c)\n",
        )
        .unwrap();
        fs::write(
            workspace.path().join("main.c"),
            "#include <stdio.h>\nint main(void) { puts(\"hello\"); return 0; }\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        for lane in ["cmake-a", "cmake-b"] {
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
                .sync_workspace_environment_component(lane, "trail/cmake-build@1", None, None)
                .unwrap();
            assert!(report.layers.is_empty());
        }
        let executable = |workdir: &Path| {
            [
                workdir.join("build/Debug/hello.exe"),
                workdir.join("build/hello.exe"),
            ]
            .into_iter()
            .find(|path| path.is_file())
            .expect("CMake did not produce hello.exe")
        };
        for lane in ["cmake-a", "cmake-b"] {
            let mounted = db.mount_fuse_cow_workdir_for_lane(lane).unwrap();
            let workdir = PathBuf::from(db.lane_workdir(lane).unwrap().workdir.unwrap());
            assert!(Command::new("cmake")
                .args(["-S", ".", "-B", "build"])
                .current_dir(&workdir)
                .status()
                .unwrap()
                .success());
            assert!(Command::new("cmake")
                .args(["--build", "build", "--config", "Debug", "--parallel", "2"])
                .current_dir(&workdir)
                .status()
                .unwrap()
                .success());
            assert!(executable(&workdir).is_file());
            let cache = fs::read_to_string(workdir.join("build/CMakeCache.txt"))
                .unwrap()
                .replace('\\', "/");
            assert!(cache.contains(&workdir.to_string_lossy().replace('\\', "/")));
            drop(mounted);
        }
        let mounted = db.mount_fuse_cow_workdir_for_lane("cmake-a").unwrap();
        let workdir_a = PathBuf::from(db.lane_workdir("cmake-a").unwrap().workdir.unwrap());
        assert!(Command::new("cmake")
            .args(["--build", "build", "--target", "clean", "--config", "Debug"])
            .current_dir(&workdir_a)
            .status()
            .unwrap()
            .success());
        assert!(![
            workdir_a.join("build/Debug/hello.exe"),
            workdir_a.join("build/hello.exe")
        ]
        .iter()
        .any(|path| path.exists()));
        drop(mounted);
        let mounted = db.mount_fuse_cow_workdir_for_lane("cmake-b").unwrap();
        let workdir_b = PathBuf::from(db.lane_workdir("cmake-b").unwrap().workdir.unwrap());
        assert!(executable(&workdir_b).is_file());
        drop(mounted);
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }
}
