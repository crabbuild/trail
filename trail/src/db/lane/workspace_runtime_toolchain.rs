use std::ffi::OsString;
use std::fs::{self, File};
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::*;

const COLIMA_VERSION: &str = "0.10.3";
const LIMA_VERSION: &str = "2.2.0";
const DOCKER_VERSION: &str = "29.7.2";
const RECEIPT_SCHEMA: u32 = 1;
const MAX_ARCHIVE_ENTRIES: usize = 8_192;
const MAX_EXPANDED_BYTES: u64 = 768 * 1024 * 1024;
const MAX_RECEIPT_BYTES: u64 = 64 * 1024;
const CONTAINED_PROFILE_RECEIPT_SCHEMA: u32 = 1;
const CONTAINED_PROFILE_CONTRACT: &str = "trail_no_host_mounts_v1";
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(10 * 60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const THIRD_PARTY_NOTICES: &[u8] =
    include_bytes!("../../../assets/runtime-toolchain/THIRD_PARTY_NOTICES.md");
const COLIMA_LICENSE: &[u8] = include_bytes!("../../../assets/runtime-toolchain/COLIMA-LICENSE");
const APACHE_LICENSE: &[u8] =
    include_bytes!("../../../assets/runtime-toolchain/APACHE-2.0-LICENSE");

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ArtifactKind {
    Executable,
    LimaArchive,
    DockerArchive,
}

#[derive(Clone, Copy, Debug)]
struct ManagedArtifact {
    name: &'static str,
    url: &'static str,
    sha256: &'static str,
    max_bytes: u64,
    kind: ArtifactKind,
}

#[derive(Clone, Copy, Debug)]
struct ManagedManifest {
    platform: &'static str,
    artifacts: &'static [ManagedArtifact],
    colima_executable_sha256: &'static str,
    limactl_executable_sha256: &'static str,
    docker_executable_sha256: &'static str,
}

const DARWIN_ARM64_ARTIFACTS: &[ManagedArtifact] = &[
    ManagedArtifact {
        name: "colima-Darwin-arm64",
        url: "https://github.com/abiosoft/colima/releases/download/v0.10.3/colima-Darwin-arm64",
        sha256: "980ad8bf61a4ca370243f4cb41401a61276dcd2c2502bee7b9b86f9250169f34",
        max_bytes: 32 * 1024 * 1024,
        kind: ArtifactKind::Executable,
    },
    ManagedArtifact {
        name: "lima-2.2.0-Darwin-arm64.tar.gz",
        url: "https://github.com/lima-vm/lima/releases/download/v2.2.0/lima-2.2.0-Darwin-arm64.tar.gz",
        sha256: "bbdef91774885a0d05f7b048c4eb89ae2bcf3a0c252ae7ca7934e63df76d93c3",
        max_bytes: 96 * 1024 * 1024,
        kind: ArtifactKind::LimaArchive,
    },
    ManagedArtifact {
        name: "docker-29.7.2.tgz",
        url: "https://download.docker.com/mac/static/stable/aarch64/docker-29.7.2.tgz",
        sha256: "b8683ed19d1f06048a496f9b8429e2c71d0b088d475b7487c054ea3666c02a3c",
        max_bytes: 96 * 1024 * 1024,
        kind: ArtifactKind::DockerArchive,
    },
];

const DARWIN_X86_64_ARTIFACTS: &[ManagedArtifact] = &[
    ManagedArtifact {
        name: "colima-Darwin-x86_64",
        url: "https://github.com/abiosoft/colima/releases/download/v0.10.3/colima-Darwin-x86_64",
        sha256: "3082737fe8a98afda11cba7d9a20b6e56fe80c6153464beda04bec630758770b",
        max_bytes: 32 * 1024 * 1024,
        kind: ArtifactKind::Executable,
    },
    ManagedArtifact {
        name: "lima-2.2.0-Darwin-x86_64.tar.gz",
        url: "https://github.com/lima-vm/lima/releases/download/v2.2.0/lima-2.2.0-Darwin-x86_64.tar.gz",
        sha256: "0d6f99c19f6e4bc3c92730c4c29d929e6927f0cb0a0ba1a84383367135a8ff31",
        max_bytes: 96 * 1024 * 1024,
        kind: ArtifactKind::LimaArchive,
    },
    ManagedArtifact {
        name: "docker-29.7.2.tgz",
        url: "https://download.docker.com/mac/static/stable/x86_64/docker-29.7.2.tgz",
        sha256: "fb1f1aa7ac7af4364165b9eadfda92e96c8ced508fca74f53079719891367438",
        max_bytes: 96 * 1024 * 1024,
        kind: ArtifactKind::DockerArchive,
    },
];

const DARWIN_ARM64_MANIFEST: ManagedManifest = ManagedManifest {
    platform: "darwin-arm64",
    artifacts: DARWIN_ARM64_ARTIFACTS,
    colima_executable_sha256: "980ad8bf61a4ca370243f4cb41401a61276dcd2c2502bee7b9b86f9250169f34",
    limactl_executable_sha256: "f19a4fca3875e1017a5285672be4a62699c1e55918fb6a7afce86a14199e10d9",
    docker_executable_sha256: "a078469d8b77683b81e1604ee35af488ef143a8a0230897f05f0839b2f42d1dd",
};

const DARWIN_X86_64_MANIFEST: ManagedManifest = ManagedManifest {
    platform: "darwin-x86_64",
    artifacts: DARWIN_X86_64_ARTIFACTS,
    colima_executable_sha256: "3082737fe8a98afda11cba7d9a20b6e56fe80c6153464beda04bec630758770b",
    limactl_executable_sha256: "a02801d546fe8f3d59fe0a8b7d8831c2e4acec06a0c61cce0badb4e781be3535",
    docker_executable_sha256: "c38429dd6b8803858e891d1c2e703ed28b9dd65e146d24f35f614049db36096e",
};

#[derive(Clone, Debug)]
pub(super) struct ColimaToolchain {
    pub(super) colima: PathBuf,
    pub(super) limactl: PathBuf,
    pub(super) docker: PathBuf,
    managed_path: Option<OsString>,
    state: ColimaStatePaths,
    pub(super) source: &'static str,
    pub(super) version: Option<String>,
    pub(super) managed_vz: bool,
}

#[derive(Clone, Debug)]
struct ColimaStatePaths {
    colima_home: PathBuf,
    lima_home: PathBuf,
    docker_config: PathBuf,
    colima_cache: PathBuf,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ManagedToolchainReceipt {
    schema: u32,
    toolchain: String,
    platform: String,
    manifest_sha256: String,
    colima_version: String,
    lima_version: String,
    docker_version: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ContainedProfileReceipt {
    schema: u32,
    profile: String,
    contract: String,
    toolchain_source: String,
    toolchain_version: Option<String>,
}

impl ColimaToolchain {
    #[cfg(test)]
    pub(super) fn for_guest_protocol_test(limactl: PathBuf, state_root: &Path) -> Self {
        Self {
            colima: limactl.clone(),
            limactl: limactl.clone(),
            docker: limactl,
            managed_path: None,
            state: ColimaStatePaths {
                colima_home: state_root.join("colima"),
                lima_home: state_root.join("lima"),
                docker_config: state_root.join("docker"),
                colima_cache: state_root.join("cache"),
            },
            source: "system",
            version: None,
            managed_vz: false,
        }
    }

    pub(super) fn resolve(allow_install: bool) -> Result<Self> {
        if let Ok(system) = Self::resolve_system() {
            return Ok(system);
        }
        let manifest = current_manifest()?;
        let cache_root = managed_cache_root()?;
        let install_dir = managed_install_dir(&cache_root, manifest);
        if let Ok(managed) = Self::resolve_managed(manifest, &install_dir) {
            return Ok(managed);
        }
        if !allow_install {
            return Err(Error::InvalidInput(
                "a complete Colima toolchain is unavailable; run `trail env runtime setup colima` to install Trail's pinned runtime tools"
                    .to_string(),
            ));
        }
        ensure_managed_host_supported()?;
        provision_managed_toolchain(manifest, &cache_root)?;
        Self::resolve_managed(manifest, &install_dir)
    }

    fn resolve_system() -> Result<Self> {
        let colima = super::workspace_environment::resolve_workspace_tool_executable("colima")?;
        let limactl = super::workspace_environment::resolve_workspace_tool_executable("limactl")?;
        let docker = super::workspace_environment::resolve_workspace_tool_executable("docker")?;
        Ok(Self {
            colima: colima.path,
            limactl: limactl.path,
            docker: docker.path,
            managed_path: None,
            state: colima_state_paths()?,
            source: "system",
            version: None,
            managed_vz: false,
        })
    }

    fn resolve_managed(manifest: &ManagedManifest, install_dir: &Path) -> Result<Self> {
        validate_managed_toolchain(manifest, install_dir)?;
        Ok(Self {
            colima: install_dir.join("bin/colima"),
            limactl: install_dir.join("bin/limactl"),
            docker: install_dir.join("bin/docker"),
            managed_path: Some(prepend_path(&install_dir.join("bin"))?),
            state: colima_state_paths()?,
            source: "trail_managed",
            version: Some(toolchain_version()),
            managed_vz: true,
        })
    }

    pub(super) fn state_is_ready(&self) -> bool {
        [
            &self.state.colima_home,
            &self.state.lima_home,
            &self.state.docker_config,
            &self.state.colima_cache,
        ]
        .into_iter()
        .all(|path| path.is_dir() && !path.is_symlink())
    }

    pub(super) fn prepare_state(&self) -> Result<()> {
        for path in [
            &self.state.colima_home,
            &self.state.lima_home,
            &self.state.docker_config,
            &self.state.colima_cache,
        ] {
            ensure_private_directory(path)?;
        }
        Ok(())
    }

    pub(super) fn colima_command(&self) -> Command {
        self.command(&self.colima)
    }

    pub(super) fn docker_command(&self) -> Command {
        let mut command = self.command(&self.docker);
        command.env_remove("DOCKER_HOST");
        command
    }

    pub(super) fn limactl_command(&self) -> Command {
        self.command(&self.limactl)
    }

    pub(super) fn record_contained_profile(&self, profile: &str) -> Result<()> {
        let directory = self.state.colima_home.join("trail-profile-receipts");
        ensure_private_directory(&directory)?;
        let receipt = ContainedProfileReceipt {
            schema: CONTAINED_PROFILE_RECEIPT_SCHEMA,
            profile: profile.to_string(),
            contract: CONTAINED_PROFILE_CONTRACT.to_string(),
            toolchain_source: self.source.to_string(),
            toolchain_version: self.version.clone(),
        };
        write_file_atomic(
            &directory.join(format!("{profile}.json")),
            &serde_json::to_vec_pretty(&receipt)?,
            false,
        )
    }

    pub(super) fn contained_profile_verified(&self, profile: &str) -> bool {
        let path = self
            .state
            .colima_home
            .join("trail-profile-receipts")
            .join(format!("{profile}.json"));
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            return false;
        };
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.len() > MAX_RECEIPT_BYTES
        {
            return false;
        }
        let Ok(bytes) = fs::read(path) else {
            return false;
        };
        let Ok(receipt) = serde_json::from_slice::<ContainedProfileReceipt>(&bytes) else {
            return false;
        };
        receipt.schema == CONTAINED_PROFILE_RECEIPT_SCHEMA
            && receipt.profile == profile
            && receipt.contract == CONTAINED_PROFILE_CONTRACT
            && receipt.toolchain_source == self.source
            && receipt.toolchain_version == self.version
    }

    fn command(&self, executable: &Path) -> Command {
        let mut command = Command::new(executable);
        command
            .env("COLIMA_HOME", &self.state.colima_home)
            .env("LIMA_HOME", &self.state.lima_home)
            .env("DOCKER_CONFIG", &self.state.docker_config)
            .env("COLIMA_CACHE_HOME", &self.state.colima_cache);
        if let Some(path) = &self.managed_path {
            command.env("PATH", path);
        }
        command
    }
}

fn current_manifest() -> Result<&'static ManagedManifest> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Ok(&DARWIN_ARM64_MANIFEST),
        ("macos", "x86_64") => Ok(&DARWIN_X86_64_MANIFEST),
        (os, arch) => Err(Error::InvalidInput(format!(
            "Trail-managed Colima tools support macOS arm64 and x86_64; `{os}-{arch}` requires system-installed colima, limactl, and docker"
        ))),
    }
}

fn ensure_managed_host_supported() -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        let output = Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .map_err(|error| {
                Error::InvalidInput(format!("could not determine the macOS version: {error}"))
            })?;
        let version = String::from_utf8_lossy(&output.stdout);
        let major = version
            .trim()
            .split('.')
            .next()
            .and_then(|value| value.parse::<u64>().ok())
            .ok_or_else(|| {
                Error::InvalidInput("could not parse the macOS product version".to_string())
            })?;
        if !output.status.success() || major < 13 {
            return Err(Error::InvalidInput(
                "Trail-managed Colima requires macOS 13 or newer for Apple's `vz` virtualization backend; install Colima, Lima, Docker, and QEMU manually on this host"
                    .to_string(),
            ));
        }
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        Err(Error::InvalidInput(
            "Trail-managed Colima requires macOS 13 or newer; install colima, limactl, and docker manually on this host"
                .to_string(),
        ))
    }
}

fn toolchain_version() -> String {
    format!("colima-{COLIMA_VERSION}+lima-{LIMA_VERSION}+docker-{DOCKER_VERSION}")
}

fn managed_install_dir(cache_root: &Path, manifest: &ManagedManifest) -> PathBuf {
    cache_root
        .join("colima")
        .join(toolchain_version())
        .join(manifest.platform)
}

fn managed_cache_root() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    let root = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library/Caches"));
    #[cfg(target_os = "windows")]
    let root = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let root = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".cache"))
        });
    root.map(|root| root.join("trail/runtime-tools"))
        .ok_or_else(|| {
            Error::InvalidInput("cannot resolve Trail's user cache directory".to_string())
        })
}

fn colima_state_paths() -> Result<ColimaStatePaths> {
    #[cfg(target_os = "macos")]
    let state = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| macos_colima_state_roots(&home));
    #[cfg(target_os = "windows")]
    let state = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .map(|root| {
            let data_root = root.join("trail/runtime");
            let lima_home = data_root.join("colima/_lima");
            (data_root, lima_home)
        });
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let state = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".local/share"))
        })
        .map(|root| {
            let data_root = root.join("trail/runtime");
            let lima_home = data_root.join("colima/_lima");
            (data_root, lima_home)
        });
    let (data_root, lima_home) = state.ok_or_else(|| {
        Error::InvalidInput("cannot resolve Trail's user data directory".to_string())
    })?;
    let colima_home = data_root.join("colima");
    Ok(ColimaStatePaths {
        lima_home,
        docker_config: data_root.join("docker"),
        colima_cache: data_root.join("cache/colima"),
        colima_home,
    })
}

#[cfg(target_os = "macos")]
fn macos_colima_state_roots(home: &Path) -> (PathBuf, PathBuf) {
    (
        home.join("Library/Application Support/trail/runtime"),
        // Lima appends the instance name plus a randomized SSH socket suffix
        // to LIMA_HOME. Keeping that root below Application Support exceeds
        // macOS's 104-byte AF_UNIX limit for Trail's workspace-scoped names.
        home.join(".trail-lima"),
    )
}

fn ensure_private_directory(path: &Path) -> Result<()> {
    if path.exists() {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(Error::InvalidInput(format!(
                "Trail runtime state path `{}` is not a real directory",
                path.display()
            )));
        }
    } else {
        fs::create_dir_all(path)?;
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn prepend_path(bin_dir: &Path) -> Result<OsString> {
    let mut paths = vec![bin_dir.to_path_buf()];
    if let Some(path) = std::env::var_os("PATH") {
        paths.extend(std::env::split_paths(&path));
    }
    std::env::join_paths(paths).map_err(|error| {
        Error::InvalidInput(format!("could not construct managed tool PATH: {error}"))
    })
}

fn provision_managed_toolchain(manifest: &ManagedManifest, cache_root: &Path) -> Result<()> {
    let client = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(DOWNLOAD_TIMEOUT)
        .user_agent(format!(
            "trail/{}/runtime-toolchain",
            env!("CARGO_PKG_VERSION")
        ))
        .build()
        .map_err(|error| {
            Error::InvalidInput(format!("could not create runtime download client: {error}"))
        })?;
    provision_managed_toolchain_with(manifest, cache_root, |artifact, destination| {
        download_artifact(&client, artifact, destination)
    })
}

fn provision_managed_toolchain_with<F>(
    manifest: &ManagedManifest,
    cache_root: &Path,
    mut fetch: F,
) -> Result<()>
where
    F: FnMut(&ManagedArtifact, &Path) -> Result<()>,
{
    let install_dir = managed_install_dir(cache_root, manifest);
    if validate_managed_toolchain(manifest, &install_dir).is_ok() {
        return Ok(());
    }
    if install_dir.exists() {
        return Err(Error::InvalidInput(format!(
            "Trail-managed runtime cache `{}` is corrupt; remove that version directory and rerun setup",
            install_dir.display()
        )));
    }
    let parent = install_dir.parent().ok_or_else(|| {
        Error::InvalidInput("managed runtime install path has no parent".to_string())
    })?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(
        ".{}.staging-{}-{}",
        manifest.platform,
        std::process::id(),
        nonce()
    ));
    if staging.exists() {
        return Err(Error::InvalidInput(
            "managed runtime staging path already exists".to_string(),
        ));
    }
    fs::create_dir(&staging)?;
    let result = install_into(manifest, &staging, &mut fetch);
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    match fs::rename(&staging, &install_dir) {
        Ok(()) => {
            super::sync_directory_strict(parent)?;
            Ok(())
        }
        Err(_error) if validate_managed_toolchain(manifest, &install_dir).is_ok() => {
            let _ = fs::remove_dir_all(&staging);
            Ok(())
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            Err(Error::Io(error))
        }
    }
}

fn install_into<F>(manifest: &ManagedManifest, staging: &Path, fetch: &mut F) -> Result<()>
where
    F: FnMut(&ManagedArtifact, &Path) -> Result<()>,
{
    fs::create_dir(staging.join("bin"))?;
    for (index, artifact) in manifest.artifacts.iter().enumerate() {
        require_https(artifact.url)?;
        let download = staging.join(format!(".download-{index}"));
        fetch(artifact, &download)?;
        verify_file_digest(&download, artifact.sha256, artifact.max_bytes)?;
        match artifact.kind {
            ArtifactKind::Executable => {
                fs::rename(&download, staging.join("bin/colima"))?;
            }
            ArtifactKind::LimaArchive => {
                unpack_lima_archive(&download, staging)?;
                fs::remove_file(&download)?;
            }
            ArtifactKind::DockerArchive => {
                unpack_docker_archive(&download, staging)?;
                fs::remove_file(&download)?;
            }
        }
    }
    let license_dir = staging.join("licenses");
    fs::create_dir(&license_dir)?;
    write_new_file(
        &license_dir.join("THIRD_PARTY_NOTICES.md"),
        THIRD_PARTY_NOTICES,
    )?;
    write_new_file(&license_dir.join("COLIMA-LICENSE"), COLIMA_LICENSE)?;
    write_new_file(&license_dir.join("APACHE-2.0-LICENSE"), APACHE_LICENSE)?;
    set_executable(&staging.join("bin/colima"))?;
    set_executable(&staging.join("bin/limactl"))?;
    set_executable(&staging.join("bin/docker"))?;
    validate_executable_digests(manifest, staging)?;
    let receipt = ManagedToolchainReceipt {
        schema: RECEIPT_SCHEMA,
        toolchain: toolchain_version(),
        platform: manifest.platform.to_string(),
        manifest_sha256: manifest_identity(manifest),
        colima_version: COLIMA_VERSION.to_string(),
        lima_version: LIMA_VERSION.to_string(),
        docker_version: DOCKER_VERSION.to_string(),
    };
    write_new_file(
        &staging.join("receipt.json"),
        &serde_json::to_vec_pretty(&receipt)?,
    )?;
    validate_managed_toolchain(manifest, staging)
}

fn download_artifact(
    client: &Client,
    artifact: &ManagedArtifact,
    destination: &Path,
) -> Result<()> {
    let response = client
        .get(artifact.url)
        .send()
        .and_then(reqwest::blocking::Response::error_for_status)
        .map_err(|error| {
            Error::InvalidInput(format!(
                "could not download managed runtime artifact `{}`: {error}",
                artifact.name
            ))
        })?;
    if response
        .content_length()
        .is_some_and(|length| length > artifact.max_bytes)
    {
        return Err(Error::InvalidInput(format!(
            "managed runtime artifact `{}` exceeds its {} MiB limit",
            artifact.name,
            artifact.max_bytes / (1024 * 1024)
        )));
    }
    let mut file = File::create(destination)?;
    let copied = io::copy(
        &mut response.take(artifact.max_bytes.saturating_add(1)),
        &mut file,
    )?;
    if copied > artifact.max_bytes {
        return Err(Error::InvalidInput(format!(
            "managed runtime artifact `{}` exceeds its {} MiB limit",
            artifact.name,
            artifact.max_bytes / (1024 * 1024)
        )));
    }
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

fn validate_managed_toolchain(manifest: &ManagedManifest, install_dir: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(install_dir).map_err(|_| {
        Error::InvalidInput("Trail-managed runtime toolchain is not installed".to_string())
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Error::InvalidInput(
            "Trail-managed runtime toolchain path is not a real directory".to_string(),
        ));
    }
    let receipt_bytes = read_bounded(&install_dir.join("receipt.json"), MAX_RECEIPT_BYTES)?;
    let receipt: ManagedToolchainReceipt = serde_json::from_slice(&receipt_bytes)?;
    if receipt.schema != RECEIPT_SCHEMA
        || receipt.toolchain != toolchain_version()
        || receipt.platform != manifest.platform
        || receipt.manifest_sha256 != manifest_identity(manifest)
        || receipt.colima_version != COLIMA_VERSION
        || receipt.lima_version != LIMA_VERSION
        || receipt.docker_version != DOCKER_VERSION
    {
        return Err(Error::InvalidInput(
            "Trail-managed runtime receipt does not match this Trail release".to_string(),
        ));
    }
    validate_executable_digests(manifest, install_dir)?;
    for path in [install_dir.join("share/lima"), install_dir.join("licenses")] {
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(Error::InvalidInput(format!(
                "managed runtime path `{}` is not a real directory",
                path.display()
            )));
        }
    }
    Ok(())
}

fn validate_executable_digests(manifest: &ManagedManifest, root: &Path) -> Result<()> {
    for (relative, expected) in [
        ("bin/colima", manifest.colima_executable_sha256),
        ("bin/limactl", manifest.limactl_executable_sha256),
        ("bin/docker", manifest.docker_executable_sha256),
    ] {
        let path = root.join(relative);
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(Error::InvalidInput(format!(
                "managed runtime executable `{relative}` is missing or unsafe"
            )));
        }
        let actual = sha256_file(&path, 256 * 1024 * 1024)?;
        if actual != expected {
            return Err(Error::InvalidInput(format!(
                "managed runtime executable `{relative}` failed SHA-256 verification"
            )));
        }
    }
    Ok(())
}

fn manifest_identity(manifest: &ManagedManifest) -> String {
    let mut digest = Sha256::new();
    digest.update(b"trail-colima-toolchain-v1\0");
    digest.update(manifest.platform.as_bytes());
    for artifact in manifest.artifacts {
        digest.update(b"\0");
        digest.update(artifact.name.as_bytes());
        digest.update(b"\0");
        digest.update(artifact.url.as_bytes());
        digest.update(b"\0");
        digest.update(artifact.sha256.as_bytes());
        digest.update(b"\0");
        digest.update(artifact.max_bytes.to_le_bytes());
        digest.update([artifact.kind as u8]);
    }
    hex::encode(digest.finalize())
}

fn verify_file_digest(path: &Path, expected: &str, max_bytes: u64) -> Result<()> {
    let actual = sha256_file(path, max_bytes)?;
    if actual == expected {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "managed runtime artifact `{}` failed SHA-256 verification",
            path.display()
        )))
    }
}

fn sha256_file(path: &Path, max_bytes: u64) -> Result<String> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > max_bytes {
        return Err(Error::InvalidInput(format!(
            "managed runtime file `{}` is missing, unsafe, or oversized",
            path.display()
        )));
    }
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut total = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(read as u64);
        if total > max_bytes {
            return Err(Error::InvalidInput(format!(
                "managed runtime file `{}` exceeds its safety limit",
                path.display()
            )));
        }
        digest.update(&buffer[..read]);
    }
    Ok(hex::encode(digest.finalize()))
}

fn unpack_lima_archive(path: &Path, destination: &Path) -> Result<()> {
    unpack_tar_gz(path, destination, |relative, entry_type| {
        if entry_type.is_symlink() && relative == Path::new("share/doc/lima/templates") {
            return Ok(ArchiveDisposition::Skip);
        }
        if entry_type.is_dir() || entry_type.is_file() {
            Ok(ArchiveDisposition::Extract(relative.to_path_buf()))
        } else {
            Err(Error::InvalidInput(format!(
                "Lima archive contains unsupported entry `{}`",
                relative.display()
            )))
        }
    })
}

fn unpack_docker_archive(path: &Path, destination: &Path) -> Result<()> {
    unpack_tar_gz(path, destination, |relative, entry_type| {
        if entry_type.is_dir() {
            Ok(ArchiveDisposition::Skip)
        } else if entry_type.is_file() && relative == Path::new("docker/docker") {
            Ok(ArchiveDisposition::Extract(PathBuf::from("bin/docker")))
        } else {
            Err(Error::InvalidInput(format!(
                "Docker CLI archive contains unexpected entry `{}`",
                relative.display()
            )))
        }
    })
}

enum ArchiveDisposition {
    Skip,
    Extract(PathBuf),
}

fn unpack_tar_gz<F>(path: &Path, destination: &Path, mut classify: F) -> Result<()>
where
    F: FnMut(&Path, tar::EntryType) -> Result<ArchiveDisposition>,
{
    let decoder = GzDecoder::new(File::open(path)?);
    let mut archive = tar::Archive::new(decoder);
    let mut entries = 0usize;
    let mut expanded = 0u64;
    for entry in archive.entries()? {
        let mut entry = entry?;
        entries = entries.saturating_add(1);
        if entries > MAX_ARCHIVE_ENTRIES {
            return Err(Error::InvalidInput(
                "managed runtime archive contains too many entries".to_string(),
            ));
        }
        let relative = safe_archive_path(&entry.path()?)?;
        if relative.as_os_str().is_empty() {
            continue;
        }
        let entry_type = entry.header().entry_type();
        let disposition = classify(&relative, entry_type)?;
        let ArchiveDisposition::Extract(output_relative) = disposition else {
            continue;
        };
        let output_relative = safe_archive_path(&output_relative)?;
        let output = destination.join(output_relative);
        if entry_type.is_dir() {
            fs::create_dir_all(&output)?;
            continue;
        }
        if !entry_type.is_file() {
            return Err(Error::InvalidInput(
                "managed runtime archive extraction accepted a non-file entry".to_string(),
            ));
        }
        expanded = expanded.saturating_add(entry.size());
        if expanded > MAX_EXPANDED_BYTES {
            return Err(Error::InvalidInput(
                "managed runtime archive exceeds its expanded size limit".to_string(),
            ));
        }
        let parent = output.parent().ok_or_else(|| {
            Error::InvalidInput("managed runtime archive output has no parent".to_string())
        })?;
        fs::create_dir_all(parent)?;
        if output.exists() {
            return Err(Error::InvalidInput(format!(
                "managed runtime archive repeats `{}`",
                output.display()
            )));
        }
        let mut file = File::create(&output)?;
        let copied = io::copy(&mut entry, &mut file)?;
        if copied != entry.size() {
            return Err(Error::InvalidInput(format!(
                "managed runtime archive entry `{}` was truncated",
                relative.display()
            )));
        }
        file.flush()?;
    }
    Ok(())
}

fn safe_archive_path(path: &Path) -> Result<PathBuf> {
    let mut safe = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::Normal(value) => safe.push(value),
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(Error::InvalidInput(format!(
                    "managed runtime archive path `{}` escapes its staging directory",
                    path.display()
                )));
            }
        }
    }
    Ok(safe)
}

fn set_executable(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(Error::InvalidInput(format!(
            "managed runtime executable `{}` is missing or unsafe",
            path.display()
        )));
    }
    #[cfg(unix)]
    fs::set_permissions(path, fs::Permissions::from_mode(0o555))?;
    Ok(())
}

fn write_new_file(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.flush()?;
    file.sync_all()?;
    Ok(())
}

fn read_bounded(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() || metadata.len() > limit {
        return Err(Error::InvalidInput(format!(
            "managed runtime receipt `{}` is missing, unsafe, or oversized",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    File::open(path)?
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::InvalidInput(
            "managed runtime receipt exceeds its safety limit".to_string(),
        ));
    }
    Ok(bytes)
}

fn require_https(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url).map_err(|error| {
        Error::InvalidInput(format!("invalid managed runtime URL `{url}`: {error}"))
    })?;
    if parsed.scheme() != "https" || parsed.host_str().is_none() {
        return Err(Error::InvalidInput(format!(
            "managed runtime URL `{url}` must use HTTPS"
        )));
    }
    Ok(())
}

fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use tar::Builder;

    fn fixture_archive(entries: &[(&str, &[u8], u32)]) -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = Builder::new(encoder);
        for (path, bytes, mode) in entries {
            let mut header = tar::Header::new_gnu();
            header.set_path(path).unwrap();
            header.set_size(bytes.len() as u64);
            header.set_mode(*mode);
            header.set_cksum();
            builder.append(&header, *bytes).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn fixture_manifest(
        colima: &'static [u8],
        lima: &'static [u8],
        docker: &'static [u8],
    ) -> (ManagedManifest, Vec<u8>, Vec<u8>) {
        let lima_archive = fixture_archive(&[
            ("bin/limactl", lima, 0o755),
            ("share/lima/guestagent", b"guest", 0o644),
            ("share/doc/lima/LICENSE", b"license", 0o644),
        ]);
        let docker_archive = fixture_archive(&[("docker/docker", docker, 0o755)]);
        let artifacts = vec![
            ManagedArtifact {
                name: "colima",
                url: "https://fixtures.invalid/colima",
                sha256: Box::leak(sha256_bytes(colima).into_boxed_str()),
                max_bytes: 1024,
                kind: ArtifactKind::Executable,
            },
            ManagedArtifact {
                name: "lima.tar.gz",
                url: "https://fixtures.invalid/lima",
                sha256: Box::leak(sha256_bytes(&lima_archive).into_boxed_str()),
                max_bytes: 1024 * 1024,
                kind: ArtifactKind::LimaArchive,
            },
            ManagedArtifact {
                name: "docker.tgz",
                url: "https://fixtures.invalid/docker",
                sha256: Box::leak(sha256_bytes(&docker_archive).into_boxed_str()),
                max_bytes: 1024 * 1024,
                kind: ArtifactKind::DockerArchive,
            },
        ];
        let artifacts = Box::leak(artifacts.into_boxed_slice());
        (
            ManagedManifest {
                platform: "fixture",
                artifacts,
                colima_executable_sha256: Box::leak(sha256_bytes(colima).into_boxed_str()),
                limactl_executable_sha256: Box::leak(sha256_bytes(lima).into_boxed_str()),
                docker_executable_sha256: Box::leak(sha256_bytes(docker).into_boxed_str()),
            },
            lima_archive,
            docker_archive,
        )
    }

    fn sha256_bytes(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    #[test]
    fn managed_install_verifies_publishes_reuses_and_rejects_corruption() {
        let cache = tempfile::tempdir().unwrap();
        let (manifest, lima_archive, docker_archive) =
            fixture_manifest(b"colima", b"limactl", b"docker");
        let mut downloads = 0usize;
        provision_managed_toolchain_with(&manifest, cache.path(), |artifact, destination| {
            downloads += 1;
            let bytes = match artifact.kind {
                ArtifactKind::Executable => b"colima".to_vec(),
                ArtifactKind::LimaArchive => lima_archive.clone(),
                ArtifactKind::DockerArchive => docker_archive.clone(),
            };
            fs::write(destination, bytes).map_err(Error::from)
        })
        .unwrap();
        assert_eq!(downloads, 3);
        provision_managed_toolchain_with(&manifest, cache.path(), |_artifact, _destination| {
            panic!("valid published toolchain must not download again")
        })
        .unwrap();

        let install = managed_install_dir(cache.path(), &manifest);
        let docker = install.join("bin/docker");
        #[cfg(unix)]
        fs::set_permissions(&docker, fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(&docker, b"tampered").unwrap();
        let error =
            provision_managed_toolchain_with(&manifest, cache.path(), |_artifact, _destination| {
                panic!("corrupt published state fails before download")
            })
            .unwrap_err();
        assert!(error.to_string().contains("cache") && error.to_string().contains("corrupt"));
    }

    #[test]
    fn managed_install_failure_never_publishes_stage() {
        let cache = tempfile::tempdir().unwrap();
        let (manifest, _lima_archive, _docker_archive) =
            fixture_manifest(b"colima", b"limactl", b"docker");
        let error =
            provision_managed_toolchain_with(&manifest, cache.path(), |artifact, destination| {
                let bytes = if artifact.kind == ArtifactKind::Executable {
                    b"wrong".as_slice()
                } else {
                    b"unused".as_slice()
                };
                fs::write(destination, bytes).map_err(Error::from)
            })
            .unwrap_err();
        assert!(error.to_string().contains("SHA-256"));
        assert!(!managed_install_dir(cache.path(), &manifest).exists());
        assert!(
            fs::read_dir(cache.path().join("colima").join(toolchain_version()))
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("staging"))
        );
    }

    #[test]
    fn concurrent_managed_installers_converge_on_one_verified_tree() {
        let cache = tempfile::tempdir().unwrap();
        let (manifest, lima_archive, docker_archive) =
            fixture_manifest(b"colima", b"limactl", b"docker");
        std::thread::scope(|scope| {
            for _ in 0..2 {
                let lima_archive = lima_archive.clone();
                let docker_archive = docker_archive.clone();
                let thread_manifest = manifest;
                let cache_path = cache.path().to_path_buf();
                scope.spawn(move || {
                    provision_managed_toolchain_with(
                        &thread_manifest,
                        &cache_path,
                        |artifact, destination| {
                            let bytes = match artifact.kind {
                                ArtifactKind::Executable => b"colima".to_vec(),
                                ArtifactKind::LimaArchive => lima_archive.clone(),
                                ArtifactKind::DockerArchive => docker_archive.clone(),
                            };
                            fs::write(destination, bytes).map_err(Error::from)
                        },
                    )
                    .unwrap();
                });
            }
        });
        validate_managed_toolchain(&manifest, &managed_install_dir(cache.path(), &manifest))
            .unwrap();
        assert!(
            fs::read_dir(cache.path().join("colima").join(toolchain_version()))
                .unwrap()
                .all(|entry| !entry
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .contains("staging"))
        );
    }

    #[test]
    fn archive_paths_reject_parent_traversal() {
        assert!(safe_archive_path(Path::new("../escape")).is_err());
        assert!(safe_archive_path(Path::new("/absolute")).is_err());
        assert_eq!(
            safe_archive_path(Path::new("./bin/limactl")).unwrap(),
            Path::new("bin/limactl")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_lima_home_leaves_room_for_workspace_profile_socket() {
        let home = Path::new("/Users/abcdefghijklmnopqrstuvwxyzabcde");
        let (data_root, lima_home) = macos_colima_state_roots(home);
        let socket = lima_home
            .join("colima-trail-0123456789ab")
            .join("ssh.sock.1234567890123456");

        assert_eq!(
            data_root,
            home.join("Library/Application Support/trail/runtime")
        );
        assert!(
            socket.as_os_str().len() < 104,
            "representative Lima SSH socket must fit macOS AF_UNIX: {}",
            socket.display()
        );
    }
}
