use super::workspace_environment::{
    resolve_workspace_tool_executable, workspace_mounted_commands_identity,
    WorkspaceEnvironmentAdapter, WorkspaceEnvironmentAdapterMetadata, WorkspaceEnvironmentCommand,
    WorkspaceEnvironmentInput, WorkspaceEnvironmentOutput, WorkspaceEnvironmentOutputPolicy,
    WorkspaceEnvironmentPlan, WorkspaceEnvironmentSandboxPolicy,
};
use super::*;

pub(crate) struct PythonVenvAdapter;

pub(crate) static PYTHON_VENV_ADAPTER: PythonVenvAdapter = PythonVenvAdapter;

const PYTHON_IDENTITY_FILES: [&str; 7] = [
    "pyproject.toml",
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
        distribution_digest: "builtin:python-venv-plan-v2",
        selectors: &["trail/python-venv@1", "python-venv", "python"],
        kind: "dependency",
        layer_adapter_name: "python-venv",
        discovery_markers: &PYTHON_IDENTITY_FILES,
        supported_operating_systems: &["linux", "macos", "windows"],
        supported_architectures: &["aarch64", "x86_64"],
        stability: "experimental",
        description: "Automatically initialized lane-private Python virtual environment at the stable mounted lane path",
    };

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

    fn plan(
        &self,
        db: &Trail,
        source_root: &ObjectId,
        component_root: &str,
    ) -> Result<WorkspaceEnvironmentPlan> {
        let component_root = normalize_python_component_root(component_root)?;
        let python = resolve_python_executable()?;
        let component_id = self.component_id(&component_root)?;
        let mount_path = join_python_path(&component_root, ".venv");
        let implementation_version = env!("CARGO_PKG_VERSION").to_string();
        let distribution_digest = "builtin:python-venv-plan-v2".to_string();
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
        mounted_args.push(".venv".to_string());
        let mounted_command = WorkspaceEnvironmentCommand {
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
                "mounted_action".to_string(),
                workspace_mounted_commands_identity(std::slice::from_ref(&mounted_command))?,
            ),
        ]);
        let mut inputs = Vec::new();
        for file in PYTHON_IDENTITY_FILES {
            let path = join_python_path(&component_root, file);
            if let Some(entry) = db.root_file_entry(source_root, &path)? {
                key_inputs.insert(format!("input:{path}"), entry.content_hash.clone());
                inputs.push(WorkspaceEnvironmentInput {
                    source_path: path.clone(),
                    staging_path: format!("project/{path}"),
                    entry,
                });
            }
        }
        if inputs.is_empty() {
            return Err(Error::InvalidInput(format!(
                "Python component `{}` has no supported dependency manifest or lockfile",
                display_python_root(&component_root)
            )));
        }
        inputs.sort_by(|left, right| left.source_path.cmp(&right.source_path));
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
                tool_versions: BTreeMap::from([(
                    "python-executable".to_string(),
                    python.identity.clone(),
                )]),
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "lane-private-host-python".to_string(),
                strategy: "python-venv-private-mounted-init-v2".to_string(),
            },
            inputs,
            source_projection: None,
            pre_commands: Vec::new(),
            // Python virtual environments commonly embed absolute interpreter
            // and prefix paths, so the host initializes this output through
            // an ephemeral candidate view mounted at the final lane path.
            command: None,
            mounted_commands: vec![mounted_command],
            caches: Vec::new(),
            external_artifacts: Vec::new(),
            runtime_resources: Vec::new(),
            sandbox_policy: WorkspaceEnvironmentSandboxPolicy::TrustedBuiltin,
            outputs: vec![WorkspaceEnvironmentOutput {
                name: "venv".to_string(),
                output_path: "private/venv".to_string(),
                mount_path,
                policy: WorkspaceEnvironmentOutputPolicy::WritablePrivate,
                create_if_missing: true,
            }],
            stale_reason:
                "Python executable, dependency manifest or lockfile, component root, platform, or adapter policy changed"
                    .to_string(),
        })
    }
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
        for _ in 0..1_000 {
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
    fn python_venv_is_keyed_private_and_initialized_at_the_mounted_lane() {
        if resolve_python_executable().is_err() {
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
            } else {
                LaneWorkdirMode::OverlayCow
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
        assert_eq!(plan.commands.len(), 1);
        assert_eq!(plan.commands[0].phase, "mounted_initialization");
        #[cfg(target_os = "macos")]
        assert_eq!(
            plan.commands[0].args,
            ["-m", "venv", "--without-pip", "--copies", ".venv"]
        );
        #[cfg(not(target_os = "macos"))]
        assert_eq!(
            plan.commands[0].args,
            ["-m", "venv", "--without-pip", ".venv"]
        );
        assert_eq!(plan.outputs[0].mount_path, ".venv");
        assert_eq!(plan.outputs[0].policy, "writable_private");
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
        assert_eq!(output.policy, "writable_private");
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
            } else {
                LaneWorkdirMode::OverlayCow
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
            } else {
                LaneWorkdirMode::OverlayCow
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
        let mounted = db.mount_overlay_cow_workdir_for_lane("python-all").unwrap();
        let workdir = PathBuf::from(db.lane_workdir("python-all").unwrap().workdir.unwrap());
        for component in ["services/api", "services/worker"] {
            let venv = workdir.join(component).join(".venv");
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
                fs::canonicalize(&venv).unwrap()
            );
            #[cfg(not(windows))]
            assert_eq!(
                String::from_utf8(prefix.stdout).unwrap().trim(),
                venv.to_string_lossy()
            );
        }
        drop(mounted);
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn real_python_venvs_embed_lane_paths_and_remain_isolated() {
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
                } else {
                    LaneWorkdirMode::OverlayCow
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
            let mounted = db.mount_overlay_cow_workdir_for_lane(lane).unwrap();
            let workdir = PathBuf::from(db.lane_workdir(lane).unwrap().workdir.unwrap());
            assert!(workdir.join(".venv/pyvenv.cfg").is_file());
            let venv_python = workdir.join(".venv/bin/python");
            let prefix = Command::new(&venv_python)
                .args(["-c", "import sys; print(sys.prefix)"])
                .current_dir(&workdir)
                .output()
                .unwrap();
            assert!(prefix.status.success());
            assert_eq!(
                String::from_utf8(prefix.stdout).unwrap().trim(),
                workdir.join(".venv").to_string_lossy()
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
        let mounted = db.mount_overlay_cow_workdir_for_lane("python-a").unwrap();
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
                LaneWorkdirMode::OverlayCow,
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

            let mounted = db.mount_overlay_cow_workdir_for_lane(lane).unwrap();
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
            let expected_prefix = fs::canonicalize(workdir.join(".venv")).unwrap();
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
        let mounted = db.mount_overlay_cow_workdir_for_lane("python-a").unwrap();
        let workdir = PathBuf::from(db.lane_workdir("python-a").unwrap().workdir.unwrap());
        assert!(workdir.join(".venv/lane-a.txt").is_file());
        drop(mounted);
        assert!(db.list_workspace_layers().unwrap().is_empty());
    }
}
