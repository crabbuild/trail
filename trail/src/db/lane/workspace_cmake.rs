use super::workspace_environment::{
    resolve_workspace_tool_executable, workspace_tool_identity_for_path,
    WorkspaceEnvironmentAdapter, WorkspaceEnvironmentAdapterMetadata,
    WorkspaceEnvironmentCacheAccess, WorkspaceEnvironmentCacheCommandBinding,
    WorkspaceEnvironmentCacheProtocol, WorkspaceEnvironmentCommandBinding,
    WorkspaceEnvironmentOutput, WorkspaceEnvironmentOutputCommandBinding,
    WorkspaceEnvironmentOutputPolicy, WorkspaceEnvironmentPlan, WorkspaceEnvironmentSandboxPolicy,
    WorkspaceEnvironmentToolCommandBinding, WORKSPACE_COMMAND_BINDING_MOUNTPOINT,
};
use super::*;
use crate::ids::sha256_hex;

pub(crate) struct CmakeBuildTreeAdapter;

pub(crate) static CMAKE_BUILD_TREE_ADAPTER: CmakeBuildTreeAdapter = CmakeBuildTreeAdapter;

static CMAKE_BUILD_TREE_ADAPTER_METADATA: WorkspaceEnvironmentAdapterMetadata =
    WorkspaceEnvironmentAdapterMetadata {
        canonical_identity: "trail/cmake-build@1",
        namespace: "trail",
        name: "cmake-build",
        contract_major: 1,
        implementation_version: env!("CARGO_PKG_VERSION"),
        distribution_digest: "builtin:cmake-build-plan-v3",
        selectors: &["trail/cmake-build@1", "cmake-build", "cmake"],
        kind: "build",
        layer_adapter_name: "cmake-build",
        discovery_markers: &["CMakeLists.txt"],
        supported_operating_systems: &["linux", "macos", "windows"],
        supported_architectures: &["aarch64", "x86_64"],
        stability: "experimental",
        description: "Lane-private CMake build tree with contained preset, Ninja, toolchain, ccache, and pinned vcpkg identity",
    };

const CMAKE_CACHE_COMMAND_BINDINGS: &[WorkspaceEnvironmentCacheCommandBinding] = &[
    WorkspaceEnvironmentCacheCommandBinding {
        cache_name: "compiler-cache",
        environment: "CCACHE_DIR",
        relative_path: "",
        required: false,
    },
    WorkspaceEnvironmentCacheCommandBinding {
        cache_name: "vcpkg-downloads",
        environment: "VCPKG_DOWNLOADS",
        relative_path: "",
        required: false,
    },
    WorkspaceEnvironmentCacheCommandBinding {
        cache_name: "vcpkg-binaries",
        environment: "VCPKG_DEFAULT_BINARY_CACHE",
        relative_path: "",
        required: false,
    },
];

const CMAKE_COMMAND_BINDINGS: &[WorkspaceEnvironmentCommandBinding] = &[
    WorkspaceEnvironmentCommandBinding {
        environment: "CMAKE_BUILD_PARALLEL_LEVEL",
        value: "2",
    },
    WorkspaceEnvironmentCommandBinding {
        environment: "VCPKG_BINARY_SOURCES",
        value: "clear;default,readwrite",
    },
    WorkspaceEnvironmentCommandBinding {
        environment: "X_VCPKG_ASSET_SOURCES",
        value: "clear;x-block-origin",
    },
    WorkspaceEnvironmentCommandBinding {
        environment: "CCACHE_BASEDIR",
        value: WORKSPACE_COMMAND_BINDING_MOUNTPOINT,
    },
    WorkspaceEnvironmentCommandBinding {
        environment: "CCACHE_NOHASHDIR",
        value: "true",
    },
];

const CMAKE_OUTPUT_COMMAND_BINDINGS: &[WorkspaceEnvironmentOutputCommandBinding] = &[
    WorkspaceEnvironmentOutputCommandBinding {
        output_name: "build-tree",
        environment: Some("TRAIL_CMAKE_BUILD_DIR"),
        relative_path: "",
        direct: true,
        prepend_path: false,
        required: true,
    },
    WorkspaceEnvironmentOutputCommandBinding {
        output_name: "build-tree",
        environment: Some("TRAIL_CMAKE_MOUNTED_BUILD_DIR"),
        relative_path: "",
        direct: false,
        prepend_path: false,
        required: true,
    },
];

const CMAKE_TOOL_COMMAND_BINDINGS: &[WorkspaceEnvironmentToolCommandBinding] = &[
    WorkspaceEnvironmentToolCommandBinding {
        programs: &["cmake"],
        environment: "TRAIL_CMAKE",
        required: true,
        prepend_path: true,
    },
    WorkspaceEnvironmentToolCommandBinding {
        programs: &["ninja"],
        environment: "TRAIL_NINJA",
        required: false,
        prepend_path: true,
    },
    WorkspaceEnvironmentToolCommandBinding {
        programs: &["ccache"],
        environment: "TRAIL_CCACHE",
        required: false,
        prepend_path: true,
    },
    WorkspaceEnvironmentToolCommandBinding {
        programs: &["vcpkg"],
        environment: "TRAIL_VCPKG",
        required: false,
        prepend_path: true,
    },
];

const MAX_CMAKE_PRESET_BYTES: u64 = 1024 * 1024;
const MAX_CMAKE_PRESET_FILES: usize = 64;
const CMAKE_PRESET_SELECTION_ENV: &str = "TRAIL_CMAKE_CONFIGURE_PRESET";
#[cfg(windows)]
const DEFAULT_C_COMPILER: &str = "cl";
#[cfg(not(windows))]
const DEFAULT_C_COMPILER: &str = "cc";
#[cfg(windows)]
const DEFAULT_CXX_COMPILER: &str = "cl";
#[cfg(not(windows))]
const DEFAULT_CXX_COMPILER: &str = "c++";

#[derive(Clone, Debug, Default)]
struct CmakePresetAuthority {
    selected: Option<String>,
    generator: Option<String>,
    toolchain_file: Option<String>,
    uses_ccache: bool,
    inputs: BTreeMap<String, String>,
    cache_variables: BTreeMap<String, String>,
}

#[derive(Clone, Debug, Default)]
struct CmakeDependencyAuthority {
    uses_vcpkg: bool,
    inputs: BTreeMap<String, String>,
    tool_versions: BTreeMap<String, String>,
}

#[derive(Clone, Debug)]
struct CmakePresetDefinition {
    name: String,
    hidden: bool,
    inherits: Vec<String>,
    generator: Option<String>,
    toolchain_file: Option<String>,
    cache_variables: BTreeMap<String, String>,
    environment: BTreeMap<String, String>,
}

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

    fn cache_command_bindings(&self) -> &'static [WorkspaceEnvironmentCacheCommandBinding] {
        CMAKE_CACHE_COMMAND_BINDINGS
    }

    fn command_bindings(&self) -> &'static [WorkspaceEnvironmentCommandBinding] {
        CMAKE_COMMAND_BINDINGS
    }

    fn output_command_bindings(&self) -> &'static [WorkspaceEnvironmentOutputCommandBinding] {
        CMAKE_OUTPUT_COMMAND_BINDINGS
    }

    fn tool_command_bindings(&self) -> &'static [WorkspaceEnvironmentToolCommandBinding] {
        CMAKE_TOOL_COMMAND_BINDINGS
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
        let distribution_digest = "builtin:cmake-build-plan-v3".to_string();
        let mount_path = join_repo_path(&component_root, "build");
        let component_id = self.component_id(&component_root)?;
        let preset = cmake_preset_authority(db, source_root, &component_root)?;
        let dependency = cmake_dependency_authority(db, source_root, &component_root, &preset)?;
        let mut inputs = BTreeMap::from([
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
            (
                "command_environment".to_string(),
                "TRAIL_CMAKE_BUILD_DIR=direct-output:build-tree;TRAIL_CMAKE_MOUNTED_BUILD_DIR=mounted-output:build-tree;TRAIL_CMAKE=tool:cmake;TRAIL_NINJA=tool?:ninja;TRAIL_CCACHE=tool?:ccache;CCACHE_DIR=cache?:compiler-cache;CCACHE_BASEDIR=lane-mount;CCACHE_NOHASHDIR=true;PATH+=tool-dirs"
                    .to_string(),
            ),
        ]);
        inputs.extend(
            preset
                .inputs
                .iter()
                .map(|(path, hash)| (format!("preset:{path}"), hash.clone())),
        );
        inputs.extend(
            dependency
                .inputs
                .iter()
                .map(|(path, hash)| (format!("cmake_dependency:{path}"), hash.clone())),
        );
        if let Some(selected) = &preset.selected {
            inputs.insert("configure_preset".to_string(), selected.clone());
        }
        if let Some(generator) = &preset.generator {
            inputs.insert("generator".to_string(), generator.clone());
        }
        if let Some(toolchain) = &preset.toolchain_file {
            inputs.insert("toolchain_file".to_string(), toolchain.clone());
        }
        let mut tool_versions = BTreeMap::from([("cmake-executable".to_string(), cmake.identity)]);
        if preset
            .generator
            .as_deref()
            .is_some_and(is_supported_ninja_generator)
        {
            let ninja = resolve_workspace_tool_executable("ninja")?;
            tool_versions.insert("ninja-executable".to_string(), ninja.identity);
        }
        if preset.selected.is_some() {
            let c_compiler = resolve_cmake_compiler(
                &preset.cache_variables,
                "CMAKE_C_COMPILER",
                DEFAULT_C_COMPILER,
            )?;
            let cxx_compiler = resolve_cmake_compiler(
                &preset.cache_variables,
                "CMAKE_CXX_COMPILER",
                DEFAULT_CXX_COMPILER,
            )?;
            tool_versions.insert("c-compiler".to_string(), c_compiler);
            tool_versions.insert("cxx-compiler".to_string(), cxx_compiler);
        }
        tool_versions.extend(dependency.tool_versions.clone());
        let mut caches = Vec::new();
        if preset.uses_ccache {
            let ccache = resolve_workspace_tool_executable("ccache")?;
            tool_versions.insert("ccache-executable".to_string(), ccache.identity.clone());
            caches.push(db.declare_workspace_environment_cache(
                self.identity(),
                "compiler-cache",
                WorkspaceEnvironmentCacheProtocol::CompilerCache,
                WorkspaceEnvironmentCacheAccess::ToolConcurrent,
                BTreeMap::from([
                    ("ccache_executable".to_string(), ccache.identity),
                    (
                        "c_compiler".to_string(),
                        tool_versions["c-compiler"].clone(),
                    ),
                    (
                        "cxx_compiler".to_string(),
                        tool_versions["cxx-compiler"].clone(),
                    ),
                    (
                        "generator".to_string(),
                        preset.generator.clone().unwrap_or_default(),
                    ),
                    ("platform".to_string(), std::env::consts::OS.to_string()),
                    (
                        "architecture".to_string(),
                        std::env::consts::ARCH.to_string(),
                    ),
                ]),
            )?);
        }
        if dependency.uses_vcpkg {
            for (name, purpose) in [
                ("vcpkg-downloads", "source-archives"),
                ("vcpkg-binaries", "binary-packages"),
            ] {
                caches.push(db.declare_workspace_environment_cache(
                    self.identity(),
                    name,
                    WorkspaceEnvironmentCacheProtocol::ContentStore,
                    WorkspaceEnvironmentCacheAccess::HostExclusive,
                    BTreeMap::from([
                        ("purpose".to_string(), purpose.to_string()),
                        (
                            "vcpkg".to_string(),
                            dependency.tool_versions["vcpkg-executable"].clone(),
                        ),
                        ("platform".to_string(), std::env::consts::OS.to_string()),
                        (
                            "architecture".to_string(),
                            std::env::consts::ARCH.to_string(),
                        ),
                    ]),
                )?);
            }
        }
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
                tool_versions,
                platform: std::env::consts::OS.to_string(),
                architecture: std::env::consts::ARCH.to_string(),
                portability_scope: "lane-private-host-tool".to_string(),
                strategy: "cmake-build-tree-private-v1".to_string(),
            },
            inputs: Vec::new(),
            resolution_inputs: Vec::new(),
            construction_seed: None,
            source_projection: None,
            pre_commands: Vec::new(),
            command: None,
            // CMakeCache.txt records absolute source and build paths. Running
            // configure against Trail's ephemeral sync candidate would make
            // the persisted private tree stale immediately after activation.
            // Configure is therefore explicit inside the stable lane mount:
            // cmake --preset <selected> -B "$TRAIL_CMAKE_BUILD_DIR".
            mounted_commands: Vec::new(),
            caches,
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
                reuse: EnvironmentReuseMode::None,
                scope: EnvironmentSharingScope::Lane,
                publish: EnvironmentPublicationTrigger::Never,
                gate: None,
                create_if_missing: true,
            }],
            stale_reason:
                "CMake executable, host platform, architecture, component root, or adapter policy changed"
                    .to_string(),
        })
    }
}

fn cmake_preset_authority(
    db: &Trail,
    source_root: &ObjectId,
    component_root: &str,
) -> Result<CmakePresetAuthority> {
    let project_presets = join_repo_path(component_root, "CMakePresets.json");
    let user_presets = join_repo_path(component_root, "CMakeUserPresets.json");
    let selection_path = join_repo_path(component_root, ".trail-cmake-preset");
    let mut pending = Vec::new();
    for path in [&project_presets, &user_presets] {
        if db.root_file_entry(source_root, path)?.is_some() {
            pending.push(path.clone());
        }
    }
    if pending.is_empty() {
        return Ok(CmakePresetAuthority::default());
    }

    let mut inputs = BTreeMap::new();
    let mut definitions = BTreeMap::<String, CmakePresetDefinition>::new();
    let mut seen = BTreeSet::new();
    while let Some(path) = pending.pop() {
        if !seen.insert(path.clone()) {
            continue;
        }
        if seen.len() > MAX_CMAKE_PRESET_FILES {
            return Err(Error::InvalidInput(format!(
                "CMake preset graph exceeds {MAX_CMAKE_PRESET_FILES} files"
            )));
        }
        let entry = db.root_file_entry(source_root, &path)?.ok_or_else(|| {
            Error::InvalidInput(format!("CMake preset include `{path}` does not exist"))
        })?;
        if entry.size_bytes > MAX_CMAKE_PRESET_BYTES {
            return Err(Error::InvalidInput(format!(
                "CMake preset file `{path}` exceeds {MAX_CMAKE_PRESET_BYTES} bytes"
            )));
        }
        let bytes = db.materialize_entry_bytes(&entry)?;
        let document: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            Error::InvalidInput(format!(
                "CMake preset file `{path}` is malformed JSON: {error}"
            ))
        })?;
        inputs.insert(path.clone(), entry.content_hash);
        if let Some(includes) = document.get("include") {
            let includes = match includes {
                serde_json::Value::String(include) => vec![include.as_str()],
                serde_json::Value::Array(values) => values
                    .iter()
                    .map(|value| {
                        value.as_str().ok_or_else(|| {
                            Error::InvalidInput(format!(
                                "CMake preset include entries in `{path}` must be strings"
                            ))
                        })
                    })
                    .collect::<Result<Vec<_>>>()?,
                _ => {
                    return Err(Error::InvalidInput(format!(
                        "CMake preset include in `{path}` must be a string or array"
                    )))
                }
            };
            for include in includes {
                if include.contains('$') {
                    return Err(Error::InvalidInput(format!(
                        "CMake preset include `{include}` in `{path}` uses a macro; Trail requires a literal contained include"
                    )));
                }
                pending.push(contained_cmake_reference(component_root, &path, include)?);
            }
        }
        let configure_presets = document
            .get("configurePresets")
            .map_or(Ok(&[][..]), |value| {
                value.as_array().map(Vec::as_slice).ok_or_else(|| {
                    Error::InvalidInput(format!("configurePresets in `{path}` must be an array"))
                })
            })?;
        for value in configure_presets {
            let definition = parse_cmake_preset_definition(value, &path)?;
            if definitions
                .insert(definition.name.clone(), definition.clone())
                .is_some()
            {
                return Err(Error::InvalidInput(format!(
                    "CMake configure preset `{}` is defined more than once",
                    definition.name
                )));
            }
        }
    }

    let committed_selection = if let Some(entry) =
        db.root_file_entry(source_root, &selection_path)?
    {
        if entry.size_bytes > 256 {
            return Err(Error::InvalidInput(format!(
                "CMake preset selection `{selection_path}` exceeds 256 bytes"
            )));
        }
        let bytes = db.materialize_entry_bytes(&entry)?;
        inputs.insert(selection_path.clone(), entry.content_hash);
        let selected = std::str::from_utf8(&bytes)
            .map_err(|_| {
                Error::InvalidInput(format!(
                    "CMake preset selection `{selection_path}` is not UTF-8"
                ))
            })?
            .trim();
        if selected.is_empty() || selected.len() > 128 || selected.chars().any(char::is_whitespace)
        {
            return Err(Error::InvalidInput(format!(
                "CMake preset selection `{selection_path}` must contain one bounded preset name"
            )));
        }
        Some(selected.to_string())
    } else {
        None
    };
    let environment_selection = std::env::var(CMAKE_PRESET_SELECTION_ENV)
        .ok()
        .map(|selected| selected.trim().to_string())
        .filter(|selected| !selected.is_empty());
    if environment_selection
        .as_ref()
        .is_some_and(|selected| selected.len() > 128 || selected.chars().any(char::is_whitespace))
    {
        return Err(Error::InvalidInput(format!(
            "{CMAKE_PRESET_SELECTION_ENV} must name one bounded configure preset"
        )));
    }
    if let (Some(committed), Some(environment)) = (&committed_selection, &environment_selection)
        && committed != environment
    {
        return Err(Error::InvalidInput(format!(
            "committed CMake preset selection `{committed}` conflicts with {CMAKE_PRESET_SELECTION_ENV}=`{environment}`"
        )));
    }
    if let Some(selected) = &environment_selection {
        inputs.insert("environment-configure-preset".to_string(), selected.clone());
    }
    let explicitly_selected = committed_selection.or(environment_selection);
    let selected = if let Some(selected) = explicitly_selected {
        let definition = definitions.get(&selected).ok_or_else(|| {
            Error::InvalidInput(format!(
                "CMake preset selection names missing configure preset `{selected}`"
            ))
        })?;
        if definition.hidden {
            return Err(Error::InvalidInput(format!(
                "CMake configure preset `{selected}` is hidden and cannot be selected"
            )));
        }
        selected
    } else {
        let visible = definitions
            .values()
            .filter(|definition| !definition.hidden)
            .map(|definition| definition.name.clone())
            .collect::<Vec<_>>();
        match visible.as_slice() {
            [selected] => selected.clone(),
            [] => {
                return Err(Error::InvalidInput(
                    "CMake preset graph has no visible configure preset".to_string(),
                ))
            }
            _ => {
                return Err(Error::InvalidInput(format!(
                    "CMake preset selection is ambiguous ({}); commit .trail-cmake-preset with one configure preset name",
                    visible.join(", ")
                )))
            }
        }
    };
    let expanded = expand_cmake_preset(&selected, &definitions, &mut BTreeSet::new())?;
    let generator = expanded.generator.ok_or_else(|| {
        Error::InvalidInput(format!(
            "CMake configure preset `{selected}` does not resolve a generator"
        ))
    })?;
    if !is_supported_ninja_generator(&generator) {
        return Err(Error::InvalidInput(format!(
            "CMake configure preset `{selected}` selects unsupported generator `{generator}`; modern preset certification currently requires Ninja or Ninja Multi-Config"
        )));
    }
    let toolchain_file = expanded
        .toolchain_file
        .or_else(|| expanded.cache_variables.get("CMAKE_TOOLCHAIN_FILE").cloned())
        .map(|reference| {
            if reference == "$env{VCPKG_ROOT}/scripts/buildsystems/vcpkg.cmake" {
                return Ok(reference);
            }
            if reference.contains('$') {
                return Err(Error::InvalidInput(format!(
                    "CMake toolchain reference `{reference}` uses an unsupported macro; Trail permits only a contained literal path or the exact pinned vcpkg host-toolchain reference"
                )));
            }
            let path = contained_cmake_component_path(component_root, &reference)?;
            let entry = db.root_file_entry(source_root, &path)?.ok_or_else(|| {
                Error::InvalidInput(format!("CMake toolchain file `{path}` does not exist"))
            })?;
            if entry.size_bytes > MAX_CMAKE_PRESET_BYTES {
                return Err(Error::InvalidInput(format!(
                    "CMake toolchain file `{path}` exceeds {MAX_CMAKE_PRESET_BYTES} bytes"
                )));
            }
            inputs.insert(path.clone(), entry.content_hash);
            Ok(path)
        })
        .transpose()?;
    let uses_ccache = ["CMAKE_C_COMPILER_LAUNCHER", "CMAKE_CXX_COMPILER_LAUNCHER"]
        .iter()
        .any(|key| {
            expanded
                .cache_variables
                .get(*key)
                .is_some_and(|value| value.split(';').any(|part| part == "ccache"))
        });
    validate_cmake_preset_environment(&selected, &expanded.environment)?;
    let expanded_identity = serde_json::json!({
        "name": selected,
        "generator": generator,
        "toolchain_file": toolchain_file,
        "cache_variables": expanded.cache_variables,
        "environment": expanded.environment,
    });
    inputs.insert(
        "expanded-configure-preset".to_string(),
        sha256_hex(&serde_json::to_vec(&expanded_identity)?),
    );
    Ok(CmakePresetAuthority {
        selected: Some(selected),
        generator: Some(generator),
        toolchain_file,
        uses_ccache,
        inputs,
        cache_variables: expanded.cache_variables,
    })
}

fn is_supported_ninja_generator(generator: &str) -> bool {
    matches!(generator, "Ninja" | "Ninja Multi-Config")
}

fn parse_cmake_preset_definition(
    value: &serde_json::Value,
    source_path: &str,
) -> Result<CmakePresetDefinition> {
    let object = value.as_object().ok_or_else(|| {
        Error::InvalidInput(format!(
            "configure preset in `{source_path}` must be an object"
        ))
    })?;
    let name = object
        .get("name")
        .and_then(serde_json::Value::as_str)
        .filter(|name| !name.is_empty() && name.len() <= 128)
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "configure preset in `{source_path}` requires one bounded name"
            ))
        })?
        .to_string();
    let inherits = match object.get("inherits") {
        None => Vec::new(),
        Some(serde_json::Value::String(parent)) => vec![parent.clone()],
        Some(serde_json::Value::Array(parents)) => parents
            .iter()
            .map(|parent| {
                parent.as_str().map(str::to_string).ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "configure preset `{name}` inherits entries must be strings"
                    ))
                })
            })
            .collect::<Result<Vec<_>>>()?,
        Some(_) => {
            return Err(Error::InvalidInput(format!(
                "configure preset `{name}` inherits must be a string or array"
            )))
        }
    };
    let mut cache_variables = BTreeMap::new();
    if let Some(values) = object.get("cacheVariables") {
        let values = values.as_object().ok_or_else(|| {
            Error::InvalidInput(format!(
                "configure preset `{name}` cacheVariables must be an object"
            ))
        })?;
        for (key, value) in values {
            let value = match value {
                serde_json::Value::String(value) => value.clone(),
                serde_json::Value::Bool(value) => value.to_string(),
                serde_json::Value::Number(value) => value.to_string(),
                serde_json::Value::Object(value) => value
                    .get("value")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| {
                        Error::InvalidInput(format!(
                            "configure preset `{name}` cache variable `{key}` has an unsupported typed value"
                        ))
                    })?
                    .to_string(),
                serde_json::Value::Null => continue,
                _ => {
                    return Err(Error::InvalidInput(format!(
                        "configure preset `{name}` cache variable `{key}` has an unsupported value"
                    )))
                }
            };
            if key.len() > 256 || value.len() > 4096 || value.contains('\0') {
                return Err(Error::InvalidInput(format!(
                    "configure preset `{name}` contains an oversized cache variable"
                )));
            }
            cache_variables.insert(key.clone(), value);
        }
    }
    let environment = parse_cmake_string_map(object.get("environment"), &name, "environment")?;
    Ok(CmakePresetDefinition {
        name,
        hidden: object
            .get("hidden")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        inherits,
        generator: object
            .get("generator")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        toolchain_file: object
            .get("toolchainFile")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        cache_variables,
        environment,
    })
}

fn parse_cmake_string_map(
    value: Option<&serde_json::Value>,
    preset_name: &str,
    field: &str,
) -> Result<BTreeMap<String, String>> {
    let Some(value) = value else {
        return Ok(BTreeMap::new());
    };
    let values = value.as_object().ok_or_else(|| {
        Error::InvalidInput(format!(
            "configure preset `{preset_name}` {field} must be an object"
        ))
    })?;
    if values.len() > 256 {
        return Err(Error::InvalidInput(format!(
            "configure preset `{preset_name}` {field} exceeds 256 entries"
        )));
    }
    let mut parsed = BTreeMap::new();
    for (key, value) in values {
        let value = value.as_str().ok_or_else(|| {
            Error::InvalidInput(format!(
                "configure preset `{preset_name}` {field} entry `{key}` must be a string"
            ))
        })?;
        if key.is_empty()
            || key.len() > 256
            || value.len() > 4096
            || key.contains('\0')
            || value.contains('\0')
        {
            return Err(Error::InvalidInput(format!(
                "configure preset `{preset_name}` contains an invalid {field} entry"
            )));
        }
        parsed.insert(key.clone(), value.to_string());
    }
    Ok(parsed)
}

fn validate_cmake_preset_environment(
    preset_name: &str,
    environment: &BTreeMap<String, String>,
) -> Result<()> {
    for (name, value) in environment {
        let normalized = name.to_ascii_uppercase();
        if matches!(
            normalized.as_str(),
            "CC" | "CXX" | "CMAKE_GENERATOR" | "CMAKE_TOOLCHAIN_FILE"
        ) {
            return Err(Error::InvalidInput(format!(
                "CMake configure preset `{preset_name}` selects toolchain state through environment `{name}`; use fingerprinted generator/compiler/toolchain fields"
            )));
        }
        if normalized.contains("TOKEN")
            || normalized.contains("SECRET")
            || normalized.contains("PASSWORD")
            || normalized.contains("CREDENTIAL")
        {
            return Err(Error::InvalidInput(format!(
                "CMake configure preset `{preset_name}` contains secret-like environment key `{name}`"
            )));
        }
        if value.contains("$penv{") || value.contains("$env{") {
            if name == "VCPKG_ROOT" && value == "$penv{VCPKG_ROOT}" {
                continue;
            }
            return Err(Error::InvalidInput(format!(
                "CMake configure preset `{preset_name}` environment `{name}` inherits unpinned host state; only VCPKG_ROOT=$penv{{VCPKG_ROOT}} is permitted and separately verified"
            )));
        }
    }
    Ok(())
}

fn expand_cmake_preset(
    name: &str,
    definitions: &BTreeMap<String, CmakePresetDefinition>,
    visiting: &mut BTreeSet<String>,
) -> Result<CmakePresetDefinition> {
    if !visiting.insert(name.to_string()) {
        return Err(Error::InvalidInput(format!(
            "CMake configure preset inheritance contains a cycle at `{name}`"
        )));
    }
    let own = definitions.get(name).ok_or_else(|| {
        Error::InvalidInput(format!(
            "CMake configure preset `{name}` inherits a missing preset"
        ))
    })?;
    if visiting.len() > MAX_CMAKE_PRESET_FILES {
        return Err(Error::InvalidInput(format!(
            "CMake preset inheritance exceeds {MAX_CMAKE_PRESET_FILES} entries"
        )));
    }
    let mut expanded = CmakePresetDefinition {
        name: own.name.clone(),
        hidden: own.hidden,
        inherits: Vec::new(),
        generator: None,
        toolchain_file: None,
        cache_variables: BTreeMap::new(),
        environment: BTreeMap::new(),
    };
    for parent in &own.inherits {
        let parent = expand_cmake_preset(parent, definitions, visiting)?;
        if expanded.generator.is_none() {
            expanded.generator = parent.generator;
        }
        if expanded.toolchain_file.is_none() {
            expanded.toolchain_file = parent.toolchain_file;
        }
        for (key, value) in parent.cache_variables {
            expanded.cache_variables.entry(key).or_insert(value);
        }
        for (key, value) in parent.environment {
            expanded.environment.entry(key).or_insert(value);
        }
    }
    if own.generator.is_some() {
        expanded.generator = own.generator.clone();
    }
    if own.toolchain_file.is_some() {
        expanded.toolchain_file = own.toolchain_file.clone();
    }
    expanded.cache_variables.extend(own.cache_variables.clone());
    expanded.environment.extend(own.environment.clone());
    visiting.remove(name);
    Ok(expanded)
}

fn contained_cmake_reference(
    component_root: &str,
    containing_file: &str,
    reference: &str,
) -> Result<String> {
    if reference.is_empty() || reference.len() > 4096 {
        return Err(Error::InvalidInput(
            "CMake preset include is empty or oversized".to_string(),
        ));
    }
    let parent = containing_file
        .rsplit_once('/')
        .map_or("", |(parent, _)| parent);
    let joined = join_repo_path(parent, reference);
    let normalized = normalize_relative_path(&joined).map_err(|error| {
        Error::InvalidInput(format!(
            "CMake preset include `{reference}` escapes its component: {error}"
        ))
    })?;
    ensure_cmake_component_path(component_root, &normalized, "preset include")?;
    Ok(normalized)
}

fn contained_cmake_component_path(component_root: &str, reference: &str) -> Result<String> {
    if reference.is_empty() || reference.len() > 4096 {
        return Err(Error::InvalidInput(
            "CMake component path is empty or oversized".to_string(),
        ));
    }
    let normalized =
        normalize_relative_path(&join_repo_path(component_root, reference)).map_err(|error| {
            Error::InvalidInput(format!(
                "CMake component path `{reference}` escapes its component: {error}"
            ))
        })?;
    ensure_cmake_component_path(component_root, &normalized, "component path")?;
    Ok(normalized)
}

fn ensure_cmake_component_path(component_root: &str, path: &str, kind: &str) -> Result<()> {
    if component_root.is_empty()
        || path == component_root
        || path.starts_with(&format!("{component_root}/"))
    {
        return Ok(());
    }
    Err(Error::InvalidInput(format!(
        "CMake {kind} `{path}` escapes component `{}`",
        display_component_root(component_root)
    )))
}

fn resolve_cmake_compiler(
    cache_variables: &BTreeMap<String, String>,
    key: &str,
    fallback: &str,
) -> Result<String> {
    let selected = cache_variables.get(key).map_or(fallback, String::as_str);
    if selected.contains('$') || selected.contains(';') || selected.contains('\0') {
        return Err(Error::InvalidInput(format!(
            "CMake compiler `{selected}` is not a literal executable"
        )));
    }
    if selected.contains('/') || selected.contains('\\') {
        let path = PathBuf::from(selected);
        if !path.is_absolute() || !path.is_file() {
            return Err(Error::InvalidInput(format!(
                "CMake compiler `{selected}` must be an existing absolute host executable or a PATH program"
            )));
        }
        workspace_tool_identity_for_path(&path)
    } else {
        Ok(resolve_workspace_tool_executable(selected)?.identity)
    }
}

fn cmake_dependency_authority(
    db: &Trail,
    source_root: &ObjectId,
    component_root: &str,
    preset: &CmakePresetAuthority,
) -> Result<CmakeDependencyAuthority> {
    for marker in ["conanfile.py", "conanfile.txt", "conan.lock"] {
        let path = join_repo_path(component_root, marker);
        if db.root_file_entry(source_root, &path)?.is_some() {
            return Err(Error::InvalidInput(format!(
                "CMake component contains `{path}`; Conan is recognized but remains unsupported until its lock/profile contract is certified"
            )));
        }
    }
    let manifest_path = join_repo_path(component_root, "vcpkg.json");
    let Some(manifest_entry) = db.root_file_entry(source_root, &manifest_path)? else {
        return Ok(CmakeDependencyAuthority::default());
    };
    if manifest_entry.size_bytes > MAX_CMAKE_PRESET_BYTES {
        return Err(Error::InvalidInput(format!(
            "vcpkg manifest `{manifest_path}` exceeds {MAX_CMAKE_PRESET_BYTES} bytes"
        )));
    }
    let manifest_bytes = db.materialize_entry_bytes(&manifest_entry)?;
    let manifest: serde_json::Value = serde_json::from_slice(&manifest_bytes).map_err(|error| {
        Error::InvalidInput(format!(
            "vcpkg manifest `{manifest_path}` is malformed JSON: {error}"
        ))
    })?;
    let configuration_path = join_repo_path(component_root, "vcpkg-configuration.json");
    let configuration = if let Some(entry) = db.root_file_entry(source_root, &configuration_path)? {
        if entry.size_bytes > MAX_CMAKE_PRESET_BYTES {
            return Err(Error::InvalidInput(format!(
                "vcpkg configuration `{configuration_path}` exceeds {MAX_CMAKE_PRESET_BYTES} bytes"
            )));
        }
        let bytes = db.materialize_entry_bytes(&entry)?;
        let document: serde_json::Value = serde_json::from_slice(&bytes).map_err(|error| {
            Error::InvalidInput(format!(
                "vcpkg configuration `{configuration_path}` is malformed JSON: {error}"
            ))
        })?;
        Some((entry, document))
    } else {
        None
    };
    let baseline = manifest
        .get("builtin-baseline")
        .and_then(serde_json::Value::as_str)
        .or_else(|| {
            configuration
                .as_ref()
                .and_then(|(_, value)| value.get("default-registry"))
                .and_then(|value| value.get("baseline"))
                .and_then(serde_json::Value::as_str)
        })
        .filter(|baseline| {
            baseline.len() == 40 && baseline.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "vcpkg manifest `{manifest_path}` requires one pinned 40-hex builtin baseline"
            ))
        })?;
    validate_vcpkg_registry_paths(
        component_root,
        configuration.as_ref().map(|(_, value)| value),
    )?;
    let toolchain = preset.toolchain_file.as_deref().ok_or_else(|| {
        Error::InvalidInput(
            "vcpkg manifest mode requires a contained or exact pinned host vcpkg CMake toolchainFile in the selected preset"
                .to_string(),
        )
    })?;
    let vcpkg = resolve_workspace_tool_executable("vcpkg")?;
    let vcpkg_host = validate_vcpkg_host_toolchain(toolchain, &vcpkg)?;
    let mut inputs = BTreeMap::from([
        (manifest_path, manifest_entry.content_hash),
        (
            "builtin-baseline".to_string(),
            baseline.to_ascii_lowercase(),
        ),
        ("toolchain-file".to_string(), toolchain.to_string()),
        ("host-toolchain".to_string(), vcpkg_host),
    ]);
    if let Some((entry, _)) = configuration {
        inputs.insert(configuration_path, entry.content_hash);
    }
    Ok(CmakeDependencyAuthority {
        uses_vcpkg: true,
        inputs,
        tool_versions: BTreeMap::from([("vcpkg-executable".to_string(), vcpkg.identity)]),
    })
}

fn validate_vcpkg_host_toolchain(
    toolchain: &str,
    vcpkg: &super::workspace_environment::ResolvedWorkspaceTool,
) -> Result<String> {
    if toolchain != "$env{VCPKG_ROOT}/scripts/buildsystems/vcpkg.cmake" {
        return Err(Error::InvalidInput(format!(
            "vcpkg manifest mode requires the exact `$env{{VCPKG_ROOT}}/scripts/buildsystems/vcpkg.cmake` host boundary; selected `{toolchain}`"
        )));
    }
    let configured_root = std::env::var_os("VCPKG_ROOT").ok_or_else(|| {
        Error::InvalidInput(
            "the selected vcpkg preset requires VCPKG_ROOT to name the verified host checkout"
                .to_string(),
        )
    })?;
    let root = fs::canonicalize(configured_root).map_err(|error| {
        Error::InvalidInput(format!("VCPKG_ROOT cannot be canonicalized: {error}"))
    })?;
    if !root.is_dir() {
        return Err(Error::InvalidInput(
            "VCPKG_ROOT is not a directory".to_string(),
        ));
    }
    let executable = fs::canonicalize(&vcpkg.path)?;
    if executable.parent() != Some(root.as_path()) {
        return Err(Error::InvalidInput(format!(
            "resolved vcpkg executable `{}` is not directly owned by VCPKG_ROOT `{}`",
            executable.display(),
            root.display()
        )));
    }
    let toolchain_path = root.join("scripts/buildsystems/vcpkg.cmake");
    let toolchain_bytes = fs::read(&toolchain_path).map_err(|error| {
        Error::InvalidInput(format!(
            "verified vcpkg toolchain `{}` cannot be read: {error}",
            toolchain_path.display()
        ))
    })?;
    if toolchain_bytes.len() as u64 > MAX_CMAKE_PRESET_BYTES {
        return Err(Error::InvalidInput(format!(
            "verified vcpkg toolchain exceeds {MAX_CMAKE_PRESET_BYTES} bytes"
        )));
    }
    let revision = Command::new("git")
        .args(["-C"])
        .arg(&root)
        .args(["rev-parse", "HEAD"])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .map_err(|error| Error::InvalidInput(format!("failed to inspect VCPKG_ROOT: {error}")))?;
    let revision = std::str::from_utf8(&revision.stdout)
        .ok()
        .map(str::trim)
        .filter(|revision| {
            revision.len() == 40 && revision.bytes().all(|byte| byte.is_ascii_hexdigit())
        })
        .ok_or_else(|| {
            Error::InvalidInput(
                "VCPKG_ROOT must be a Git checkout pinned at one 40-hex revision".to_string(),
            )
        })?;
    let status = Command::new("git")
        .args(["-C"])
        .arg(&root)
        .args(["status", "--porcelain", "--untracked-files=no"])
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .output()
        .map_err(|error| Error::InvalidInput(format!("failed to verify VCPKG_ROOT: {error}")))?;
    if !status.status.success() || !status.stdout.is_empty() {
        return Err(Error::InvalidInput(
            "VCPKG_ROOT must be a clean pinned Git checkout".to_string(),
        ));
    }
    Ok(format!(
        "vcpkg-host:{revision}:toolchain-sha256:{}:executable:{}",
        sha256_hex(&toolchain_bytes),
        vcpkg.identity
    ))
}

fn validate_vcpkg_registry_paths(
    component_root: &str,
    configuration: Option<&serde_json::Value>,
) -> Result<()> {
    let Some(configuration) = configuration else {
        return Ok(());
    };
    let mut registries = configuration
        .get("registries")
        .and_then(serde_json::Value::as_array)
        .map_or_else(Vec::new, |values| values.iter().collect::<Vec<_>>());
    if let Some(default) = configuration.get("default-registry") {
        registries.push(default);
    }
    if registries.len() > 256 {
        return Err(Error::InvalidInput(
            "vcpkg configuration exceeds 256 registries".to_string(),
        ));
    }
    for registry in registries {
        let Some(object) = registry.as_object() else {
            return Err(Error::InvalidInput(
                "vcpkg registry entries must be objects".to_string(),
            ));
        };
        if let Some(path) = object.get("path").and_then(serde_json::Value::as_str) {
            contained_cmake_component_path(component_root, path)?;
        }
        if object.get("kind").and_then(serde_json::Value::as_str) == Some("filesystem")
            && object.get("path").is_none()
        {
            return Err(Error::InvalidInput(
                "vcpkg filesystem registry requires a contained path".to_string(),
            ));
        }
        if object.get("kind").and_then(serde_json::Value::as_str) == Some("git") {
            let repository = object
                .get("repository")
                .and_then(serde_json::Value::as_str)
                .filter(|repository| !repository.is_empty() && repository.len() <= 4096);
            let baseline = object
                .get("baseline")
                .and_then(serde_json::Value::as_str)
                .filter(|baseline| {
                    baseline.len() == 40 && baseline.bytes().all(|byte| byte.is_ascii_hexdigit())
                });
            if repository.is_none() || baseline.is_none() {
                return Err(Error::InvalidInput(
                    "vcpkg Git registries require a bounded repository and pinned 40-hex baseline"
                        .to_string(),
                ));
            }
        }
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

    fn cmake_authority_fixture(files: &[(&str, &str)]) -> (tempfile::TempDir, Trail, ObjectId) {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("CMakeLists.txt"),
            "cmake_minimum_required(VERSION 3.24)\nproject(example)\n",
        )
        .unwrap();
        for (path, contents) in files {
            let path = workspace.path().join(path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, contents).unwrap();
        }
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let root = db.resolve_branch_ref("main").unwrap().root_id;
        (workspace, db, root)
    }

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
    fn cmake_presets_expand_contained_includes_toolchain_and_ccache() {
        let (_workspace, db, root) = cmake_authority_fixture(&[
            (
                "CMakePresets.json",
                r#"{
                    "version": 6,
                    "include": ["cmake/base.json"],
                    "configurePresets": [{
                        "name": "trail",
                        "inherits": "base",
                        "toolchainFile": "cmake/toolchain.cmake",
                        "cacheVariables": {"CMAKE_CXX_COMPILER_LAUNCHER": "ccache"}
                    }]
                }"#,
            ),
            (
                "cmake/base.json",
                r#"{
                    "version": 6,
                    "configurePresets": [{"name": "base", "hidden": true, "generator": "Ninja"}]
                }"#,
            ),
            ("cmake/toolchain.cmake", "set(CMAKE_SYSTEM_NAME Darwin)\n"),
        ]);
        let authority = cmake_preset_authority(&db, &root, "").unwrap();
        assert_eq!(authority.selected.as_deref(), Some("trail"));
        assert_eq!(authority.generator.as_deref(), Some("Ninja"));
        assert_eq!(
            authority.toolchain_file.as_deref(),
            Some("cmake/toolchain.cmake")
        );
        assert!(authority.uses_ccache);
        assert!(authority.inputs.contains_key("CMakePresets.json"));
        assert!(authority.inputs.contains_key("cmake/base.json"));
        assert!(authority.inputs.contains_key("cmake/toolchain.cmake"));
        assert!(authority.inputs.contains_key("expanded-configure-preset"));
    }

    #[test]
    fn cmake_presets_fail_closed_on_ambiguity_escape_and_cycles() {
        let (_workspace, db, root) = cmake_authority_fixture(&[(
            "CMakePresets.json",
            r#"{"version":6,"configurePresets":[
                {"name":"one","generator":"Ninja"},
                {"name":"two","generator":"Ninja"}
            ]}"#,
        )]);
        let error = cmake_preset_authority(&db, &root, "").unwrap_err();
        assert!(error.to_string().contains("ambiguous"));

        let (_workspace, db, root) = cmake_authority_fixture(&[(
            "CMakePresets.json",
            r#"{"version":6,"include":"../outside.json","configurePresets":[]}"#,
        )]);
        let error = cmake_preset_authority(&db, &root, "").unwrap_err();
        assert!(error.to_string().contains("escapes"));

        let (_workspace, db, root) = cmake_authority_fixture(&[(
            "CMakePresets.json",
            r#"{"version":6,"configurePresets":[
                {"name":"one","inherits":"two","generator":"Ninja"},
                {"name":"two","inherits":"one","hidden":true}
            ]}"#,
        )]);
        let error = cmake_preset_authority(&db, &root, "").unwrap_err();
        assert!(error.to_string().contains("cycle"));
    }

    #[test]
    fn explicit_cmake_preset_selection_is_identity_bearing() {
        let (_workspace, db, root) = cmake_authority_fixture(&[
            (
                "CMakePresets.json",
                r#"{"version":6,"configurePresets":[
                    {"name":"debug","generator":"Ninja"},
                    {"name":"release","generator":"Ninja"}
                ]}"#,
            ),
            (".trail-cmake-preset", "release\n"),
        ]);
        let authority = cmake_preset_authority(&db, &root, "").unwrap();
        assert_eq!(authority.selected.as_deref(), Some("release"));
        assert!(authority.inputs.contains_key(".trail-cmake-preset"));
    }

    #[test]
    fn vcpkg_requires_a_pinned_baseline_before_tool_resolution() {
        let (_workspace, db, root) = cmake_authority_fixture(&[(
            "vcpkg.json",
            r#"{"name":"example","version-string":"0.1.0","dependencies":["zlib"]}"#,
        )]);
        let error = cmake_dependency_authority(&db, &root, "", &CmakePresetAuthority::default())
            .unwrap_err();
        assert!(error.to_string().contains("pinned 40-hex builtin baseline"));
    }

    #[test]
    fn conan_is_visible_but_fails_closed() {
        let (_workspace, db, root) =
            cmake_authority_fixture(&[("conan.lock", r#"{"version":"0.5"}"#)]);
        let error = cmake_dependency_authority(&db, &root, "", &CmakePresetAuthority::default())
            .unwrap_err();
        assert!(error.to_string().contains("Conan is recognized"));
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
        assert_eq!(
            plan.outputs[0].policy,
            EnvironmentOutputPolicy::WritablePrivate
        );
        assert!(plan.tools.contains_key("cmake-executable"));
        let source_root = db.resolve_branch_ref("main").unwrap().root_id;
        let raw_plan = CMAKE_BUILD_TREE_ADAPTER
            .plan(&db, &source_root, "")
            .unwrap();
        let identity = super::workspace_environment::workspace_environment_identity_contract_v3(
            &raw_plan,
            super::workspace_environment::workspace_environment_artifact_contract_digest(&raw_plan)
                .unwrap(),
        )
        .unwrap();
        assert!(!identity.source_closure_complete);
        assert!(!identity.portability_certified);
        assert_eq!(identity.trust_scope, "builtin");
        assert_eq!(
            raw_plan.outputs[0].policy,
            WorkspaceEnvironmentOutputPolicy::WritablePrivate
        );
        assert!(raw_plan.caches.is_empty());
        let report = db
            .sync_workspace_environment_component("cmake", "trail/cmake-build@1", None, None)
            .unwrap();
        assert!(report.layers.is_empty());
        assert_eq!(
            report.generation.components[0].outputs[0].policy,
            EnvironmentOutputPolicy::WritablePrivate
        );
        assert!(report.generation.components[0].outputs[0]
            .layer_id
            .is_none());
        let generation_id = report.generation.generation_id.clone();
        drop(db);
        let reopened = Trail::open(workspace.path()).unwrap();
        let active = reopened
            .active_environment_generation("cmake")
            .unwrap()
            .unwrap();
        assert_eq!(active.generation_id, generation_id);
        let environment = reopened
            .lane_workspace_environment("cmake")
            .unwrap()
            .into_iter()
            .collect::<BTreeMap<_, _>>();
        assert!(Path::new(&environment["TRAIL_CMAKE_BUILD_DIR"]).is_dir());
        assert!(environment["TRAIL_CMAKE_MOUNTED_BUILD_DIR"].ends_with("/build"));
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
            let environment = db
                .lane_workspace_environment(lane)
                .unwrap()
                .into_iter()
                .collect::<BTreeMap<_, _>>();
            let build_dir = PathBuf::from(&environment["TRAIL_CMAKE_BUILD_DIR"]);
            let view = db.lane_workspace_view(lane).unwrap().unwrap();
            assert_eq!(build_dir, Path::new(&view.generated_upper).join("build"));
            assert!(!build_dir.starts_with(&workdir));
            let configured = Command::new("cmake")
                .arg("-S")
                .arg(".")
                .arg("-B")
                .arg(&build_dir)
                .args(["-G", "Unix Makefiles"])
                .current_dir(&workdir)
                .status()
                .unwrap();
            assert!(configured.success());
            let built = Command::new("cmake")
                .arg("--build")
                .arg(&build_dir)
                .args(["--parallel", "2"])
                .current_dir(&workdir)
                .status()
                .unwrap();
            assert!(built.success());
            assert!(build_dir.join("hello").is_file());
            let cache = fs::read_to_string(build_dir.join("CMakeCache.txt")).unwrap();
            assert!(cache.contains(workdir.to_string_lossy().as_ref()));
            drop(mounted);
        }

        #[cfg(target_os = "macos")]
        let mounted = db.mount_nfs_cow_workdir_for_lane("cmake-a").unwrap();
        #[cfg(target_os = "linux")]
        let mounted = db.mount_fuse_cow_workdir_for_lane("cmake-a").unwrap();
        let workdir_a = PathBuf::from(db.lane_workdir("cmake-a").unwrap().workdir.unwrap());
        let build_dir_a = db
            .lane_workspace_environment("cmake-a")
            .unwrap()
            .into_iter()
            .collect::<BTreeMap<_, _>>()["TRAIL_CMAKE_BUILD_DIR"]
            .clone();
        let cleaned = Command::new("cmake")
            .args(["--build", &build_dir_a, "--target", "clean"])
            .current_dir(&workdir_a)
            .status()
            .unwrap();
        assert!(cleaned.success());
        assert!(!Path::new(&build_dir_a).join("hello").exists());
        drop(mounted);

        #[cfg(target_os = "macos")]
        let mounted = db.mount_nfs_cow_workdir_for_lane("cmake-b").unwrap();
        #[cfg(target_os = "linux")]
        let mounted = db.mount_fuse_cow_workdir_for_lane("cmake-b").unwrap();
        let build_dir_b = db
            .lane_workspace_environment("cmake-b")
            .unwrap()
            .into_iter()
            .collect::<BTreeMap<_, _>>()["TRAIL_CMAKE_BUILD_DIR"]
            .clone();
        assert!(Path::new(&build_dir_b).join("hello").is_file());
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
