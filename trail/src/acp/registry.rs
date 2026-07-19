use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bzip2::read::BzDecoder;
use flate2::read::GzDecoder;
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use tar::Archive;
use zip::ZipArchive;

use super::{built_in_acp_relay_command, command_in_path, AcpProviderLaunch};
use crate::model::AcpProviderProfile;
use crate::{Error, Result};

const ACP_REGISTRY_URL: &str =
    "https://cdn.agentclientprotocol.com/registry/v1/latest/registry.json";
const REGISTRY_CACHE_FILE: &str = "registry-v1.json";
const MAX_REGISTRY_BYTES: u64 = 2 * 1024 * 1024;
const MAX_BINARY_BYTES: u64 = 1024 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RegistryIndex {
    version: String,
    #[serde(default)]
    agents: Vec<RegistryAgent>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RegistryAgent {
    id: String,
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    distribution: RegistryDistribution,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
struct RegistryDistribution {
    #[serde(default)]
    binary: BTreeMap<String, RegistryBinaryDistribution>,
    npx: Option<RegistryPackageDistribution>,
    uvx: Option<RegistryPackageDistribution>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RegistryPackageDistribution {
    package: String,
    #[serde(default)]
    args: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct RegistryBinaryDistribution {
    archive: String,
    cmd: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
}

pub(super) fn registry_provider_profiles(
    cache_dir: Option<&Path>,
) -> Result<Vec<AcpProviderProfile>> {
    let registry = load_registry(cache_dir)?;
    let mut profiles = super::acp_provider_profiles();
    let mut known = profiles
        .iter()
        .map(|profile| profile.agent.to_ascii_lowercase())
        .collect::<BTreeSet<_>>();

    let mut agents = registry.agents;
    agents.sort_by(|left, right| left.id.cmp(&right.id));
    for agent in agents {
        if known.insert(agent.id.to_ascii_lowercase()) {
            profiles.push(registry_profile(&agent));
        }
    }
    profiles.sort_by(|left, right| left.agent.cmp(&right.agent));
    Ok(profiles)
}

pub(super) fn resolve_registry_provider(
    requested_agent: &str,
    cache_dir: Option<&Path>,
) -> Result<AcpProviderLaunch> {
    let registry = load_registry(cache_dir)?;
    let requested = requested_agent.trim().to_ascii_lowercase();
    let agent = registry
        .agents
        .iter()
        .find(|agent| agent.id.eq_ignore_ascii_case(&requested))
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "unknown ACP agent `{requested_agent}`; run `trail agent acp status` to see built-in and registry agents"
            ))
        })?;
    let profile = registry_profile(agent);
    let (upstream_command, upstream_env) = resolve_registry_launch(agent, cache_dir)?;
    Ok(AcpProviderLaunch {
        profile,
        upstream_command,
        upstream_env,
    })
}

pub(super) fn registry_provider_profile(
    requested_agent: &str,
    cache_dir: Option<&Path>,
) -> Result<AcpProviderProfile> {
    let registry = load_registry(cache_dir)?;
    let requested = requested_agent.trim().to_ascii_lowercase();
    registry
        .agents
        .iter()
        .find(|agent| agent.id.eq_ignore_ascii_case(&requested))
        .map(registry_profile)
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "unknown ACP agent `{requested_agent}`; run `trail agent acp status` to see built-in and registry agents"
            ))
        })
}

fn registry_profile(agent: &RegistryAgent) -> AcpProviderProfile {
    let platform = current_platform();
    let npx_ready = agent.distribution.npx.is_some() && command_in_path("npx");
    let uvx_ready = agent.distribution.uvx.is_some() && command_in_path("uvx");
    let binary_ready = agent.distribution.binary.contains_key(&platform);
    let available = npx_ready || uvx_ready || binary_ready;
    let mut notes = Vec::new();
    if npx_ready {
        notes.push("launches through npx from the ACP registry".to_string());
    } else if uvx_ready {
        notes.push("launches through uvx from the ACP registry".to_string());
    } else if binary_ready {
        notes.push(format!(
            "downloads the registry binary for {platform} on first launch"
        ));
    } else {
        notes.push(format!(
            "no usable registry launcher for {platform}; install npx or uvx, or use a supported binary platform"
        ));
    }
    if !agent.description.trim().is_empty() {
        notes.push(summarize_description(&agent.description));
    }
    AcpProviderProfile {
        agent: agent.id.clone(),
        display_name: agent.name.clone(),
        available,
        relay_command: built_in_acp_relay_command(&agent.id),
        notes,
        supports_acp: true,
        // The registry establishes ACP support, but does not advertise MCP capability.
        supports_mcp: false,
        supports_terminal: false,
        default_terminal_command: None,
    }
}

fn resolve_registry_launch(
    agent: &RegistryAgent,
    cache_dir: Option<&Path>,
) -> Result<(Vec<String>, BTreeMap<String, String>)> {
    if let Some(distribution) = &agent.distribution.npx
        && command_in_path("npx")
    {
        let mut command = vec!["npx".to_string(), "--yes".to_string()];
        command.push(distribution.package.clone());
        command.extend(distribution.args.clone());
        return Ok((command, BTreeMap::new()));
    }
    if let Some(distribution) = &agent.distribution.uvx
        && command_in_path("uvx")
    {
        let mut command = vec!["uvx".to_string(), distribution.package.clone()];
        command.extend(distribution.args.clone());
        return Ok((command, BTreeMap::new()));
    }

    let platform = current_platform();
    if let Some(distribution) = agent.distribution.binary.get(&platform) {
        let command_path = install_registry_binary(agent, distribution, cache_dir)?;
        let mut command = vec![command_path.to_string_lossy().to_string()];
        command.extend(distribution.args.clone());
        return Ok((command, distribution.env.clone()));
    }

    Err(Error::InvalidInput(format!(
        "ACP registry agent `{}` cannot launch on {platform}; install `npx` or `uvx`, or choose an agent with a matching binary distribution",
        agent.id
    )))
}

fn load_registry(cache_dir: Option<&Path>) -> Result<RegistryIndex> {
    match fetch_registry() {
        Ok(registry) => {
            if let Some(cache_dir) = cache_dir {
                let _ = cache_registry(cache_dir, &registry);
            }
            Ok(registry)
        }
        Err(fetch_error) => {
            let Some(cache_path) = cache_dir.map(registry_cache_path) else {
                return Err(fetch_error);
            };
            let bytes = fs::read(&cache_path).map_err(|_| fetch_error)?;
            parse_registry(&bytes).map_err(|cache_error| {
                Error::InvalidInput(format!(
                    "could not fetch the ACP registry and cached registry `{}` is invalid: {cache_error}",
                    cache_path.display()
                ))
            })
        }
    }
}

fn fetch_registry() -> Result<RegistryIndex> {
    let client = Client::builder()
        .timeout(Duration::from_secs(10))
        .user_agent("trail-acp-registry/1")
        .build()
        .map_err(|error| {
            Error::InvalidInput(format!("could not create ACP registry client: {error}"))
        })?;
    let response = client
        .get(ACP_REGISTRY_URL)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| Error::InvalidInput(format!("could not fetch ACP registry: {error}")))?;
    read_limited(response, MAX_REGISTRY_BYTES).and_then(|bytes| parse_registry(&bytes))
}

fn parse_registry(bytes: &[u8]) -> Result<RegistryIndex> {
    let registry: RegistryIndex = serde_json::from_slice(bytes)?;
    if registry.version.trim().is_empty() || registry.agents.is_empty() {
        return Err(Error::InvalidInput(
            "ACP registry is missing a version or agents".to_string(),
        ));
    }
    let mut ids = BTreeSet::new();
    for agent in &registry.agents {
        validate_registry_agent(agent)?;
        if !ids.insert(agent.id.to_ascii_lowercase()) {
            return Err(Error::InvalidInput(format!(
                "ACP registry contains duplicate agent id `{}`",
                agent.id
            )));
        }
    }
    Ok(registry)
}

fn validate_registry_agent(agent: &RegistryAgent) -> Result<()> {
    if !valid_registry_id(&agent.id)
        || agent.name.trim().is_empty()
        || agent.version.trim().is_empty()
    {
        return Err(Error::InvalidInput(format!(
            "ACP registry has an invalid agent entry `{}`",
            agent.id
        )));
    }
    let has_distribution = agent.distribution.npx.is_some()
        || agent.distribution.uvx.is_some()
        || !agent.distribution.binary.is_empty();
    if !has_distribution {
        return Err(Error::InvalidInput(format!(
            "ACP registry agent `{}` has no supported distribution",
            agent.id
        )));
    }
    for distribution in [
        agent.distribution.npx.as_ref(),
        agent.distribution.uvx.as_ref(),
    ]
    .into_iter()
    .flatten()
    {
        if distribution.package.trim().is_empty() || !valid_command_value(&distribution.package) {
            return Err(Error::InvalidInput(format!(
                "ACP registry agent `{}` has an invalid package name",
                agent.id
            )));
        }
        validate_command_args(agent, &distribution.args)?;
    }
    for (platform, distribution) in &agent.distribution.binary {
        if !valid_platform(platform)
            || distribution.archive.trim().is_empty()
            || distribution.cmd.trim().is_empty()
        {
            return Err(Error::InvalidInput(format!(
                "ACP registry agent `{}` has an invalid binary distribution",
                agent.id
            )));
        }
        require_https_url(&distribution.archive, "binary archive")?;
        let _ = safe_relative_path(&distribution.cmd)?;
        validate_command_args(agent, &distribution.args)?;
        for (key, value) in &distribution.env {
            if !valid_environment_key(key) || !valid_command_value(value) {
                return Err(Error::InvalidInput(format!(
                    "ACP registry agent `{}` has an invalid binary environment variable",
                    agent.id
                )));
            }
        }
    }
    Ok(())
}

fn validate_command_args(agent: &RegistryAgent, args: &[String]) -> Result<()> {
    if args.iter().all(|argument| valid_command_value(argument)) {
        return Ok(());
    }
    Err(Error::InvalidInput(format!(
        "ACP registry agent `{}` has an invalid command argument",
        agent.id
    )))
}

fn cache_registry(cache_dir: &Path, registry: &RegistryIndex) -> Result<()> {
    let cache_path = registry_cache_path(cache_dir);
    let parent = cache_path
        .parent()
        .ok_or_else(|| Error::InvalidInput("ACP registry cache path has no parent".to_string()))?;
    fs::create_dir_all(parent)?;
    let bytes = serde_json::to_vec(registry)?;
    let temporary = parent.join(format!(".{REGISTRY_CACHE_FILE}.tmp-{}", nonce()));
    fs::write(&temporary, bytes)?;
    match fs::rename(&temporary, &cache_path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            Err(Error::Io(error))
        }
    }
}

fn registry_cache_path(cache_dir: &Path) -> PathBuf {
    cache_dir.join("acp").join(REGISTRY_CACHE_FILE)
}

fn install_registry_binary(
    agent: &RegistryAgent,
    distribution: &RegistryBinaryDistribution,
    cache_dir: Option<&Path>,
) -> Result<PathBuf> {
    let cache_dir = cache_dir.ok_or_else(|| {
        Error::InvalidInput(
            "ACP registry binary installation requires a Trail workspace cache".to_string(),
        )
    })?;
    let install_dir = cache_dir
        .join("acp")
        .join("agents")
        .join(&agent.id)
        .join(safe_cache_component(&agent.version))
        .join(current_platform());
    let command_relative = safe_relative_path(&distribution.cmd)?;
    let command_path = install_dir.join(&command_relative);
    if command_path.is_file() {
        return Ok(command_path);
    }

    let parent = install_dir.parent().ok_or_else(|| {
        Error::InvalidInput("ACP registry install path has no parent".to_string())
    })?;
    fs::create_dir_all(parent)?;
    let staging = parent.join(format!(
        ".{}-{}.staging-{}",
        agent.id,
        safe_cache_component(&agent.version),
        nonce()
    ));
    fs::create_dir_all(&staging)?;
    let result = install_registry_binary_into(agent, distribution, &staging, &command_relative);
    if let Err(error) = result {
        let _ = fs::remove_dir_all(&staging);
        return Err(error);
    }
    match fs::rename(&staging, &install_dir) {
        Ok(()) => Ok(command_path),
        Err(_error) if command_path.is_file() => {
            let _ = fs::remove_dir_all(&staging);
            Ok(command_path)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(&staging);
            Err(Error::Io(error))
        }
    }
}

fn install_registry_binary_into(
    agent: &RegistryAgent,
    distribution: &RegistryBinaryDistribution,
    staging: &Path,
    command_relative: &Path,
) -> Result<()> {
    require_https_url(&distribution.archive, "binary archive")?;
    let downloaded = staging.join("registry-download");
    download_to(&distribution.archive, &downloaded)?;
    let archive_name = archive_name(&distribution.archive);
    if archive_name.ends_with(".zip") {
        unpack_zip(&downloaded, staging)?;
    } else if archive_name.ends_with(".tar.gz") || archive_name.ends_with(".tgz") {
        unpack_tar(GzDecoder::new(File::open(&downloaded)?), staging)?;
    } else if archive_name.ends_with(".tar.bz2") || archive_name.ends_with(".tbz2") {
        unpack_tar(BzDecoder::new(File::open(&downloaded)?), staging)?;
    } else {
        let target = staging.join(command_relative);
        let parent = target.parent().ok_or_else(|| {
            Error::InvalidInput("ACP registry binary command has no parent".to_string())
        })?;
        fs::create_dir_all(parent)?;
        fs::rename(&downloaded, &target)?;
    }
    let _ = fs::remove_file(&downloaded);
    let command_path = staging.join(command_relative);
    if !command_path.is_file() {
        return Err(Error::InvalidInput(format!(
            "ACP registry archive for `{}` did not contain `{}`",
            agent.id, distribution.cmd
        )));
    }
    make_executable(&command_path)?;
    Ok(())
}

fn download_to(url: &str, destination: &Path) -> Result<()> {
    let client = Client::builder()
        .timeout(Duration::from_secs(90))
        .user_agent("trail-acp-registry/1")
        .build()
        .map_err(|error| {
            Error::InvalidInput(format!("could not create ACP download client: {error}"))
        })?;
    let response = client
        .get(url)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| Error::InvalidInput(format!("could not download ACP binary: {error}")))?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_BINARY_BYTES)
    {
        return Err(Error::InvalidInput(format!(
            "ACP registry download exceeds the {} MiB safety limit",
            MAX_BINARY_BYTES / (1024 * 1024)
        )));
    }
    let mut file = File::create(destination)?;
    let bytes = io::copy(
        &mut response.take(MAX_BINARY_BYTES.saturating_add(1)),
        &mut file,
    )?;
    if bytes > MAX_BINARY_BYTES {
        return Err(Error::InvalidInput(format!(
            "ACP registry download exceeds the {} MiB safety limit",
            MAX_BINARY_BYTES / (1024 * 1024)
        )));
    }
    file.flush()?;
    Ok(())
}

fn read_limited(reader: impl Read, limit: u64) -> Result<Vec<u8>> {
    let mut reader = reader.take(limit.saturating_add(1));
    let mut bytes = Vec::new();
    reader.read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(Error::InvalidInput(format!(
            "ACP registry download exceeds the {} MiB safety limit",
            limit / (1024 * 1024)
        )));
    }
    Ok(bytes)
}

fn unpack_tar(reader: impl Read, destination: &Path) -> Result<()> {
    let mut archive = Archive::new(reader);
    let mut extracted_bytes = 0_u64;
    for entry in archive.entries().map_err(Error::Io)? {
        let mut entry = entry.map_err(Error::Io)?;
        let entry_type = entry.header().entry_type();
        let path = entry.path().map_err(Error::Io)?;
        let relative = safe_relative_path(&path.to_string_lossy())?;
        let target = destination.join(relative);
        if entry_type.is_dir() {
            fs::create_dir_all(target)?;
        } else if entry_type.is_file() {
            extracted_bytes = extracted_bytes
                .checked_add(entry.header().size().map_err(Error::Io)?)
                .ok_or_else(|| {
                    Error::InvalidInput("ACP registry archive is too large to extract".to_string())
                })?;
            if extracted_bytes > MAX_BINARY_BYTES {
                return Err(Error::InvalidInput(format!(
                    "ACP registry archive expands beyond the {} MiB safety limit",
                    MAX_BINARY_BYTES / (1024 * 1024)
                )));
            }
            let parent = target.parent().ok_or_else(|| {
                Error::InvalidInput("ACP archive file has no parent directory".to_string())
            })?;
            fs::create_dir_all(parent)?;
            entry.unpack(&target).map_err(Error::Io)?;
        } else {
            return Err(Error::InvalidInput(
                "ACP registry archives may contain only regular files and directories".to_string(),
            ));
        }
    }
    Ok(())
}

fn unpack_zip(archive_path: &Path, destination: &Path) -> Result<()> {
    let file = File::open(archive_path)?;
    let mut archive = ZipArchive::new(file).map_err(|error| {
        Error::InvalidInput(format!("invalid ACP registry zip archive: {error}"))
    })?;
    let mut extracted_bytes = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| {
            Error::InvalidInput(format!("could not read ACP registry zip archive: {error}"))
        })?;
        let relative = entry.enclosed_name().ok_or_else(|| {
            Error::InvalidInput("ACP registry zip contains an unsafe path".to_string())
        })?;
        let target = destination.join(relative);
        if entry.is_dir() {
            fs::create_dir_all(target)?;
            continue;
        }
        extracted_bytes = extracted_bytes.checked_add(entry.size()).ok_or_else(|| {
            Error::InvalidInput("ACP registry archive is too large to extract".to_string())
        })?;
        if extracted_bytes > MAX_BINARY_BYTES {
            return Err(Error::InvalidInput(format!(
                "ACP registry archive expands beyond the {} MiB safety limit",
                MAX_BINARY_BYTES / (1024 * 1024)
            )));
        }
        let parent = target.parent().ok_or_else(|| {
            Error::InvalidInput("ACP archive file has no parent directory".to_string())
        })?;
        fs::create_dir_all(parent)?;
        let mut output = File::create(&target)?;
        io::copy(&mut entry, &mut output)?;
        output.flush()?;
        #[cfg(unix)]
        if let Some(mode) = entry.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&target, fs::Permissions::from_mode(mode))?;
        }
    }
    Ok(())
}

fn make_executable(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = fs::metadata(path)?.permissions().mode() | 0o700;
        fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    }
    Ok(())
}

fn safe_relative_path(value: &str) -> Result<PathBuf> {
    let normalized = value.replace('\\', "/");
    if normalized.starts_with('/') || normalized.contains(':') {
        return Err(Error::InvalidInput(format!(
            "ACP registry path `{value}` must be relative"
        )));
    }
    let mut path = PathBuf::new();
    for component in normalized.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                return Err(Error::InvalidInput(format!(
                    "ACP registry path `{value}` escapes its install directory"
                )));
            }
            component => path.push(component),
        }
    }
    if path.as_os_str().is_empty() {
        return Err(Error::InvalidInput(
            "ACP registry path cannot be empty".to_string(),
        ));
    }
    Ok(path)
}

fn valid_registry_id(value: &str) -> bool {
    !value.is_empty()
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
}

fn valid_platform(value: &str) -> bool {
    matches!(
        value,
        "darwin-aarch64"
            | "darwin-x86_64"
            | "linux-aarch64"
            | "linux-x86_64"
            | "windows-aarch64"
            | "windows-x86_64"
    )
}

fn valid_command_value(value: &str) -> bool {
    !value.contains('\0')
}

fn valid_environment_key(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .enumerate()
            .all(|(index, character)| match index {
                0 => character == '_' || character.is_ascii_alphabetic(),
                _ => character == '_' || character.is_ascii_alphanumeric(),
            })
}

fn current_platform() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "windows",
        "linux" => "linux",
        other => other,
    };
    let architecture = std::env::consts::ARCH;
    format!("{os}-{architecture}")
}

fn archive_name(url: &str) -> String {
    url.split('?').next().unwrap_or(url).to_ascii_lowercase()
}

fn require_https_url(value: &str, label: &str) -> Result<()> {
    if !value.starts_with("https://") {
        return Err(Error::InvalidInput(format!(
            "ACP registry {label} must use HTTPS"
        )));
    }
    Ok(())
}

fn safe_cache_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.') {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn summarize_description(description: &str) -> String {
    const LIMIT: usize = 120;
    let description = description.trim();
    if description.chars().count() <= LIMIT {
        return description.to_string();
    }
    format!("{}…", description.chars().take(LIMIT).collect::<String>())
}

fn nonce() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_registry_and_exposes_every_agent_profile() {
        let registry = parse_registry(
            br#"{
                "version":"1.0.0",
                "agents":[
                    {"id":"example-npx","name":"Example npm","version":"1.2.3","distribution":{"npx":{"package":"example-acp@1.2.3","args":["--acp"]}}},
                    {"id":"example-binary","name":"Example binary","version":"1.0.0","distribution":{"binary":{"darwin-aarch64":{"archive":"https://example.com/example.tar.gz","cmd":"./bin/example","args":["serve"],"env":{"EXAMPLE_ACP":"1"}}}}}
                ]
            }"#,
        )
        .unwrap();
        assert_eq!(registry.agents.len(), 2);
        let profile = registry_profile(&registry.agents[0]);
        assert_eq!(
            profile.relay_command,
            ["trail", "acp", "relay", "example-npx"]
        );
        assert!(profile.supports_acp);
        assert!(!profile.supports_mcp);
    }

    #[test]
    fn rejects_unsafe_registry_binary_paths_and_urls() {
        assert!(safe_relative_path("../agent").is_err());
        assert!(safe_relative_path("C:/agent").is_err());
        assert!(require_https_url("http://example.com/agent", "binary archive").is_err());
        assert_eq!(
            safe_relative_path("./bin/agent").unwrap(),
            PathBuf::from("bin/agent")
        );
    }
}
