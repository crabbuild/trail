use std::collections::BTreeSet;

use serde::Deserialize;
use trail_environment_adapter_sdk::{
    serve_once, AdapterCache, AdapterCacheProtocol, AdapterCommand, AdapterExternalArtifact,
    AdapterOperation, AdapterOutput, AdapterPlanV2, AdapterResponse, AdapterResult,
    DiscoveredComponent, PinnedFile, PROTOCOL_V2,
};

const NIX_BUILDER_IMAGE: &str =
    "nixos/nix@sha256:286285edfc390096bd7e8aada40c5044dadff1eb0b60f28b193eef7ed52e5925";
const NIX_VERSION: &str = "2.29.1";
const NIX_PLATFORM: &str = "linux/arm64";

fn main() {
    if let Err(error) = serve_once(|request| {
        let result = if request.protocol != PROTOCOL_V2 {
            AdapterResult::Error {
                code: "unsupported_protocol".into(),
                message: "ecosystem build adapters require trail.environment-adapter/v2".into(),
            }
        } else {
            match &request.operation {
                AdapterOperation::Discover { files, .. } => AdapterResult::Discovered {
                    component: marker(&request.adapter_identity, files).map(|_| {
                        DiscoveredComponent::new(
                            format!(
                                "external-build.{}",
                                ecosystem_name(&request.adapter_identity).unwrap_or("unknown")
                            ),
                            if request.adapter_identity == "trail-examples/nix@1" {
                                "external"
                            } else {
                                "compiler-results"
                            },
                        )
                    }),
                },
                AdapterOperation::Plan {
                    component_id,
                    files,
                    ..
                } => plan(&request.adapter_identity, component_id, files),
            }
        };
        AdapterResponse::for_request(&request, result)
    }) {
        eprintln!("ecosystem-build-adapter: {error}");
        std::process::exit(1);
    }
}

fn ecosystem_name(identity: &str) -> Option<&'static str> {
    match identity {
        "trail-examples/bazel@1" => Some("bazel"),
        "trail-examples/gradle@1" => Some("gradle"),
        "trail-examples/maven@1" => Some("maven"),
        "trail-examples/nix@1" => Some("nix"),
        _ => None,
    }
}

fn marker<'a>(identity: &str, files: &'a [PinnedFile]) -> Option<&'a PinnedFile> {
    let name = match identity {
        "trail-examples/bazel@1" => "trail.bazel.toml",
        "trail-examples/gradle@1" => "trail.gradle.toml",
        "trail-examples/maven@1" => "trail.maven.toml",
        "trail-examples/nix@1" => "trail.nix.toml",
        _ => return None,
    };
    files.iter().find(|file| file.path == name)
}

fn plan(identity: &str, component_id: &str, files: &[PinnedFile]) -> AdapterResult {
    if marker(identity, files).is_none() {
        return AdapterResult::Error {
            code: "missing_marker".into(),
            message: "certification marker is missing".into(),
        };
    }
    if identity == "trail-examples/nix@1" {
        return match nix_plan(component_id, files) {
            Ok(plan) => AdapterResult::PlannedV2 { plan },
            Err(message) => AdapterResult::Error {
                code: "invalid_nix_certification".into(),
                message,
            },
        };
    }
    let inputs = files
        .iter()
        .map(|file| file.path.clone())
        .collect::<Vec<_>>();
    let builder = AdapterPlanV2::builder(component_id.to_string(), "compiler-results")
        .identity_inputs(inputs)
        .semantic_input("certification_contract", "offline-process-tree-v1");
    let plan = match identity {
        "trail-examples/bazel@1" => builder
            .cache(
                AdapterCache::host_exclusive("repository", AdapterCacheProtocol::ContentStore)
                    .compatibility_dimension("tool", "bazel")
                    .environment_variable("TRAIL_BAZEL_REPOSITORY_CACHE", "."),
            )
            .cache(
                AdapterCache::host_exclusive("disk", AdapterCacheProtocol::CompilerCache)
                    .compatibility_dimension("tool", "bazel")
                    .environment_variable("TRAIL_BAZEL_DISK_CACHE", "."),
            )
            .staging_command(
                AdapterCommand::new(
                    "bazel",
                    [
                        "--batch",
                        "--output_user_root=trail-bazel-output",
                        "test",
                        "--repository_cache={trail-cache:repository}",
                        "--disk_cache={trail-cache:disk}",
                        "--repository_disable_download",
                        "--symlink_prefix=trail-bazel-output/links/",
                        "//...",
                    ],
                )
                .identity_args(["--version"])
                .sandboxed_process_tree(),
            )
            .output(AdapterOutput::writable_private(
                "output-root",
                "trail-bazel-output",
                ".bazel-trail-output",
            ))
            .stale_reason("Bazel source, module/lock/config, tool, platform, or adapter changed")
            .build(),
        "trail-examples/gradle@1" => builder
            .cache(
                AdapterCache::host_exclusive("user-home", AdapterCacheProtocol::ContentStore)
                    .compatibility_dimension("tool", "gradle")
                    .environment_variable("GRADLE_USER_HOME", "."),
            )
            .staging_command(
                AdapterCommand::new(
                    "gradle",
                    [
                        "--offline",
                        "--no-daemon",
                        "--project-cache-dir",
                        "trail-gradle-project-cache",
                        "build",
                        "trailTest",
                    ],
                )
                .environment_variable(
                    "JAVA_OPTS",
                    "-XX:MaxMetaspaceSize=384m -XX:+HeapDumpOnOutOfMemoryError -Xms256m -Xmx512m -Dfile.encoding=UTF-8 -Duser.country=CA -Duser.language=en -Duser.variant",
                )
                .identity_args(["--version"])
                .sandboxed_process_tree(),
            )
            .output(AdapterOutput::writable_private("build", "build", "build"))
            .output(AdapterOutput::writable_private(
                "project-cache",
                "trail-gradle-project-cache",
                ".gradle",
            ))
            .stale_reason(
                "Gradle source, wrapper/locks/catalog/settings, tool, platform, or adapter changed",
            )
            .build(),
        "trail-examples/maven@1" => builder
            .cache(
                AdapterCache::host_exclusive("repository", AdapterCacheProtocol::ContentStore)
                    .compatibility_dimension("tool", "maven")
                    .environment_variable("TRAIL_MAVEN_REPOSITORY", "."),
            )
            .staging_command(
                AdapterCommand::new(
                    "mvn",
                    [
                        "--batch-mode",
                        "--offline",
                        "-Dmaven.repo.local={trail-cache:repository}",
                        "--log-file",
                        "trail-maven-logs/maven.log",
                        "clean",
                        "test",
                    ],
                )
                .identity_args(["--version"])
                .sandboxed_process_tree(),
            )
            .output(AdapterOutput::writable_private(
                "target", "target", "target",
            ))
            .output(AdapterOutput::writable_private(
                "logs",
                "trail-maven-logs",
                ".trail-maven-logs",
            ))
            .stale_reason("Maven source, POM/lock/settings, tool, platform, or adapter changed")
            .build(),
        _ => {
            return AdapterResult::Error {
                code: "unsupported_adapter".into(),
                message: "adapter identity is not an ecosystem certification package".into(),
            };
        }
    };
    match plan {
        Ok(plan) => AdapterResult::PlannedV2 { plan },
        Err(error) => AdapterResult::Error {
            code: "invalid_plan".into(),
            message: error.to_string(),
        },
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NixCertificationMarker {
    schema: String,
    locked: bool,
    pure: bool,
    flake_lock_sha256: String,
    nix_version: String,
    builder_image: String,
    platform: String,
    artifacts: Vec<NixStoreArtifact>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NixStoreArtifact {
    name: String,
    store_path: String,
    nar_sha256: String,
}

fn nix_plan(component_id: &str, files: &[PinnedFile]) -> Result<AdapterPlanV2, String> {
    let marker = marker("trail-examples/nix@1", files)
        .ok_or_else(|| "Nix certification marker is missing".to_string())?;
    let marker_text = std::str::from_utf8(&marker.content)
        .map_err(|_| "Nix certification marker must be UTF-8".to_string())?;
    let marker: NixCertificationMarker = toml::from_str(marker_text)
        .map_err(|error| format!("cannot decode Nix certification marker: {error}"))?;
    if marker.schema != "trail.nix-store/v1" {
        return Err("Nix certification marker has an unsupported schema".to_string());
    }
    if !marker.locked || !marker.pure {
        return Err("Nix certification requires locked = true and pure = true".to_string());
    }
    if marker.nix_version != NIX_VERSION
        || marker.builder_image != NIX_BUILDER_IMAGE
        || marker.platform != NIX_PLATFORM
    {
        return Err("Nix tool, pinned builder image, or platform identity changed".to_string());
    }
    let lock = files
        .iter()
        .find(|file| file.path == "flake.lock")
        .ok_or_else(|| "Nix certification requires a pinned flake.lock".to_string())?;
    let expected_lock_digest = normalize_sha256(&marker.flake_lock_sha256)
        .ok_or_else(|| "flake_lock_sha256 must be a lowercase SHA-256 digest".to_string())?;
    let actual_lock_digest = normalize_sha256(&lock.content_hash)
        .ok_or_else(|| "pinned flake.lock has an invalid content digest".to_string())?;
    if expected_lock_digest != actual_lock_digest {
        return Err("flake_lock_sha256 does not match the pinned flake.lock".to_string());
    }
    if marker.artifacts.len() != 2 {
        return Err(
            "Nix certification must declare exactly package and check artifacts".to_string(),
        );
    }
    let mut names = BTreeSet::new();
    let mut artifacts = Vec::with_capacity(3);
    artifacts.push(AdapterExternalArtifact::pinned_oci_image(
        "nix-builder",
        NIX_BUILDER_IMAGE,
        NIX_PLATFORM,
    ));
    for artifact in marker.artifacts {
        if !matches!(artifact.name.as_str(), "package" | "check")
            || !names.insert(artifact.name.clone())
        {
            return Err(
                "Nix certification artifact names must be unique package and check".to_string(),
            );
        }
        if !valid_nix_store_path(&artifact.store_path) {
            return Err(format!(
                "Nix certification artifact `{}` has an invalid store path",
                artifact.name
            ));
        }
        let digest = normalize_sha256(&artifact.nar_sha256).ok_or_else(|| {
            format!(
                "Nix certification artifact `{}` has an invalid NAR SHA-256 digest",
                artifact.name
            )
        })?;
        artifacts.push(AdapterExternalArtifact::verified_external(
            artifact.name,
            "nix",
            artifact.store_path,
            format!("sha256:{digest}"),
            NIX_PLATFORM,
        ));
    }
    if names != BTreeSet::from(["check".to_string(), "package".to_string()]) {
        return Err("Nix certification must include package and check artifacts".to_string());
    }

    AdapterPlanV2::builder(component_id.to_string(), "external")
        .identity_inputs(files.iter().map(|file| file.path.clone()))
        .semantic_input("certification_contract", "pure-locked-nix-store-v1")
        .semantic_input("flake_lock_sha256", format!("sha256:{expected_lock_digest}"))
        .semantic_input("nix_version", NIX_VERSION)
        .semantic_input("builder_image", NIX_BUILDER_IMAGE)
        .semantic_input("platform", NIX_PLATFORM)
        .external_artifacts(artifacts)
        .output(AdapterOutput::writable_private(
            "profile",
            "trail-nix-profile",
            ".trail-nix-profile",
        ))
        .output(AdapterOutput::writable_private(
            "state",
            "trail-nix-state",
            ".trail-nix-state",
        ))
        .stale_reason(
            "Nix source, flake lock, pure builder, immutable store identities, platform, or adapter changed",
        )
        .build()
        .map_err(|error| error.to_string())
}

fn normalize_sha256(value: &str) -> Option<&str> {
    let value = value.strip_prefix("sha256:").unwrap_or(value);
    (value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)))
    .then_some(value)
}

fn valid_nix_store_path(path: &str) -> bool {
    let Some(name) = path.strip_prefix("/nix/store/") else {
        return false;
    };
    let Some((hash, package)) = name.split_once('-') else {
        return false;
    };
    !package.is_empty()
        && !package.contains('/')
        && hash.len() == 32
        && hash
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && package
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"+._=-".contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use trail_environment_adapter_sdk::AdapterAction;

    fn marker_file(path: &str) -> PinnedFile {
        PinnedFile {
            path: path.to_string(),
            content_hash: "sha256:fixture".to_string(),
            executable: false,
            content: b"schema = 1\n".to_vec(),
        }
    }

    fn nix_files(locked: bool, pure: bool, lock_digest: &str, store_path: &str) -> Vec<PinnedFile> {
        let marker = format!(
            r#"schema = "trail.nix-store/v1"
locked = {locked}
pure = {pure}
flake_lock_sha256 = "sha256:{lock_digest}"
nix_version = "{NIX_VERSION}"
builder_image = "{NIX_BUILDER_IMAGE}"
platform = "{NIX_PLATFORM}"

[[artifacts]]
name = "package"
store_path = "{store_path}"
nar_sha256 = "sha256:{lock_digest}"

[[artifacts]]
name = "check"
store_path = "/nix/store/22222222222222222222222222222222-certification-check"
nar_sha256 = "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
"#
        );
        vec![
            PinnedFile {
                path: "trail.nix.toml".to_string(),
                content_hash: "sha256:marker".to_string(),
                executable: false,
                content: marker.into_bytes(),
            },
            PinnedFile {
                path: "flake.lock".to_string(),
                content_hash:
                    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                        .to_string(),
                executable: false,
                content: b"{}\n".to_vec(),
            },
        ]
    }

    #[test]
    fn maven_uses_the_conventional_private_target_and_offline_cache() {
        let AdapterResult::PlannedV2 { plan } = plan(
            "trail-examples/maven@1",
            "external-build.maven",
            &[marker_file("trail.maven.toml")],
        ) else {
            panic!("Maven fixture did not produce a v2 plan");
        };
        assert_eq!(plan.outputs.len(), 2);
        let target = plan
            .outputs
            .iter()
            .find(|output| output.name == "target")
            .unwrap();
        assert_eq!(target.source, "target");
        assert_eq!(target.target, "target");
        let logs = plan
            .outputs
            .iter()
            .find(|output| output.name == "logs")
            .unwrap();
        assert_eq!(logs.source, "trail-maven-logs");
        assert_eq!(logs.target, ".trail-maven-logs");
        let AdapterAction::Staging(command) = &plan.actions[0] else {
            panic!("Maven fixture did not produce a staging command");
        };
        assert!(command.process_tree);
        assert!(command.args.iter().any(|argument| argument == "--offline"));
        assert!(command
            .args
            .windows(2)
            .any(|arguments| arguments == ["clean", "test"]));
        assert!(command
            .args
            .windows(2)
            .any(|arguments| { arguments == ["--log-file", "trail-maven-logs/maven.log"] }));
        assert!(command
            .args
            .iter()
            .any(|argument| argument.contains("{trail-cache:repository}")));
        assert!(!command
            .args
            .iter()
            .any(|argument| argument.contains("project.build.directory")));
    }

    #[test]
    fn bazel_places_startup_options_before_the_test_command() {
        let AdapterResult::PlannedV2 { plan } = plan(
            "trail-examples/bazel@1",
            "external-build.bazel",
            &[marker_file("trail.bazel.toml")],
        ) else {
            panic!("Bazel fixture did not produce a v2 plan");
        };
        let AdapterAction::Staging(command) = &plan.actions[0] else {
            panic!("Bazel fixture did not produce a staging command");
        };
        let test_index = command
            .args
            .iter()
            .position(|argument| argument == "test")
            .unwrap();
        assert!(command.args[..test_index]
            .iter()
            .any(|argument| argument.starts_with("--output_user_root=")));
        assert!(command.args[test_index + 1..]
            .iter()
            .any(|argument| argument.starts_with("--repository_cache=")));
    }

    #[test]
    fn gradle_is_offline_and_matches_launcher_jvm_options_without_a_daemon() {
        let AdapterResult::PlannedV2 { plan } = plan(
            "trail-examples/gradle@1",
            "external-build.gradle",
            &[marker_file("trail.gradle.toml")],
        ) else {
            panic!("Gradle fixture did not produce a v2 plan");
        };
        let AdapterAction::Staging(command) = &plan.actions[0] else {
            panic!("Gradle fixture did not produce a staging command");
        };
        assert!(command.process_tree);
        assert!(command.args.iter().any(|argument| argument == "--offline"));
        assert!(command
            .args
            .iter()
            .any(|argument| argument == "--no-daemon"));
        assert_eq!(command.args.last().map(String::as_str), Some("trailTest"));
        let java_options = command.environment.get("JAVA_OPTS").unwrap();
        assert!(java_options.contains("-Xms256m"));
        assert!(java_options.contains("-Xmx512m"));
        assert!(java_options.contains("-Dfile.encoding=UTF-8"));
    }

    #[test]
    fn nix_records_only_pinned_store_artifacts_and_private_client_state() {
        let files = nix_files(
            true,
            true,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "/nix/store/11111111111111111111111111111111-certification-package",
        );
        let AdapterResult::PlannedV2 { plan } =
            plan("trail-examples/nix@1", "external-build.nix", &files)
        else {
            panic!("Nix fixture did not produce a v2 plan");
        };
        assert_eq!(plan.kind, "external");
        assert!(plan.actions.is_empty());
        assert!(plan.caches.is_empty());
        assert_eq!(plan.external_artifacts.len(), 3);
        assert!(plan.external_artifacts.iter().any(|artifact| {
            artifact.name == "nix-builder"
                && artifact.reference == NIX_BUILDER_IMAGE
                && artifact.platform == NIX_PLATFORM
        }));
        assert!(plan.external_artifacts.iter().any(|artifact| {
            artifact.name == "package"
                && artifact.provider == "nix"
                && artifact.reference.starts_with("/nix/store/")
        }));
        assert_eq!(plan.outputs.len(), 2);
        assert!(plan.outputs.iter().all(|output| {
            output.policy == trail_environment_adapter_sdk::AdapterOutputPolicy::WritablePrivate
                && output.reuse == trail_environment_adapter_sdk::AdapterReuseMode::None
                && output.scope == trail_environment_adapter_sdk::AdapterSharingScope::Lane
                && output.publish == trail_environment_adapter_sdk::AdapterPublicationTrigger::Never
        }));
    }

    #[test]
    fn nix_rejects_unlocked_impure_or_mismatched_certification() {
        for files in [
            nix_files(
                false,
                true,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "/nix/store/11111111111111111111111111111111-package",
            ),
            nix_files(
                true,
                false,
                "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                "/nix/store/11111111111111111111111111111111-package",
            ),
            nix_files(
                true,
                true,
                "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
                "/nix/store/11111111111111111111111111111111-package",
            ),
        ] {
            let AdapterResult::Error { code, .. } =
                plan("trail-examples/nix@1", "external-build.nix", &files)
            else {
                panic!("invalid Nix fixture was accepted");
            };
            assert_eq!(code, "invalid_nix_certification");
        }
    }

    #[test]
    fn nix_rejects_malformed_store_identity() {
        let files = nix_files(
            true,
            true,
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "/tmp/not-a-nix-store-path",
        );
        let AdapterResult::Error { message, .. } =
            plan("trail-examples/nix@1", "external-build.nix", &files)
        else {
            panic!("malformed Nix store identity was accepted");
        };
        assert!(message.contains("invalid store path"));
    }
}
