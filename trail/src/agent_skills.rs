//! Installation of Trail-owned agent skills for supported coding agents.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use walkdir::WalkDir;

use crate::{Error, Result};

const INSTALL_MANIFEST: &str = ".trail-install.json";
const INSTALL_SCHEMA: &str = "trail.agent_skill_install";
const INSTALL_VERSION: u16 = 1;
const MAX_INSTALLED_SKILL_BYTES: u64 = 4 * 1024 * 1024;
const TRAIL_LANES_SKILL: &str = "trail-lanes";

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct BundledAsset {
    relative_path: &'static str,
    bytes: &'static [u8],
}

const BUNDLED_ASSETS: &[BundledAsset] = &[
    BundledAsset {
        relative_path: "SKILL.md",
        bytes: include_bytes!("../assets/skills/trail-lanes/SKILL.md"),
    },
    BundledAsset {
        relative_path: "agents/openai.yaml",
        bytes: include_bytes!("../assets/skills/trail-lanes/agents/openai.yaml"),
    },
    BundledAsset {
        relative_path: "references/concurrent-agents.md",
        bytes: include_bytes!("../assets/skills/trail-lanes/references/concurrent-agents.md"),
    },
    BundledAsset {
        relative_path: "references/worker-lifecycle.md",
        bytes: include_bytes!("../assets/skills/trail-lanes/references/worker-lifecycle.md"),
    },
];

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentSkillProvider {
    Codex,
    Claude,
}

impl AgentSkillProvider {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSkillInstallAction {
    Create,
    Update,
    Noop,
}

#[derive(Clone, Debug)]
pub struct AgentSkillInstallRequest<'a> {
    pub provider: AgentSkillProvider,
    /// Provider configuration root, such as `$CODEX_HOME` or `~/.claude`.
    pub config_root: &'a Path,
    pub force: bool,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentSkillInstallReport {
    pub provider: AgentSkillProvider,
    pub skill: String,
    pub path: PathBuf,
    pub action: AgentSkillInstallAction,
    pub files: Vec<String>,
    pub dry_run: bool,
    pub restart_required: bool,
}

#[derive(Debug, Deserialize, Serialize)]
struct InstallManifest {
    schema: String,
    version: u16,
    provider: AgentSkillProvider,
    skill: String,
    content_digest: String,
}

/// Install or update Trail's focused lane skill beneath a provider configuration root.
pub fn install_agent_skills(
    request: AgentSkillInstallRequest<'_>,
) -> Result<AgentSkillInstallReport> {
    if !request.config_root.is_absolute() {
        return Err(Error::InvalidPath {
            path: request.config_root.display().to_string(),
            reason: "agent configuration root must be absolute".to_string(),
        });
    }
    let skills_root = request.config_root.join("skills");
    let target = skills_root.join(TRAIL_LANES_SKILL);
    let desired_digest = bundled_digest();
    let action = inspect_install_target(&target, request.provider, &desired_digest, request.force)?;

    if !request.dry_run && action != AgentSkillInstallAction::Noop {
        publish_installation(&skills_root, &target, request.provider, &desired_digest)?;
    }

    Ok(AgentSkillInstallReport {
        provider: request.provider,
        skill: TRAIL_LANES_SKILL.to_string(),
        path: target,
        action,
        files: BUNDLED_ASSETS
            .iter()
            .map(|asset| asset.relative_path.to_string())
            .collect(),
        dry_run: request.dry_run,
        restart_required: true,
    })
}

fn inspect_install_target(
    target: &Path,
    provider: AgentSkillProvider,
    desired_digest: &str,
    force: bool,
) -> Result<AgentSkillInstallAction> {
    let metadata = match fs::symlink_metadata(target) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(AgentSkillInstallAction::Create);
        }
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Error::InvalidPath {
            path: target.display().to_string(),
            reason: "agent skill target must be a real directory, not a symlink or file"
                .to_string(),
        });
    }

    let manifest_path = target.join(INSTALL_MANIFEST);
    let manifest = match read_install_manifest(&manifest_path) {
        Ok(manifest) => manifest,
        Err(_) if force => return Ok(AgentSkillInstallAction::Update),
        Err(error) => return Err(error),
    };
    if manifest.is_none() && !force {
        return Err(Error::InvalidInput(format!(
            "agent skill target `{}` is not owned by Trail; rerun with --force to replace it",
            target.display()
        )));
    }
    if let Some(manifest) = manifest.as_ref() {
        let valid_owner = manifest.schema == INSTALL_SCHEMA
            && manifest.version == INSTALL_VERSION
            && manifest.provider == provider
            && manifest.skill == TRAIL_LANES_SKILL;
        if !valid_owner && !force {
            return Err(Error::InvalidInput(format!(
                "agent skill target `{}` has an incompatible Trail ownership manifest; rerun with --force to replace it",
                target.display()
            )));
        }
        if valid_owner {
            let current_digest = installed_digest(target)?;
            if current_digest != manifest.content_digest && !force {
                return Err(Error::InvalidInput(format!(
                    "agent skill target `{}` contains local edits; preserve them or rerun with --force",
                    target.display()
                )));
            }
            if current_digest == *desired_digest {
                return Ok(AgentSkillInstallAction::Noop);
            }
        }
    }
    Ok(AgentSkillInstallAction::Update)
}

fn read_install_manifest(path: &Path) -> Result<Option<InstallManifest>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(Error::InvalidPath {
                path: path.display().to_string(),
                reason:
                    "agent skill ownership manifest must be a real file, not a symlink or directory"
                        .to_string(),
            });
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    }
    match fs::read(path) {
        Ok(bytes) => {
            if bytes.len() as u64 > MAX_INSTALLED_SKILL_BYTES {
                return Err(Error::InvalidInput(format!(
                    "agent skill ownership manifest `{}` exceeds {} bytes",
                    path.display(),
                    MAX_INSTALLED_SKILL_BYTES
                )));
            }
            Ok(Some(serde_json::from_slice(&bytes)?))
        }
        Err(error) => Err(error.into()),
    }
}

fn publish_installation(
    skills_root: &Path,
    target: &Path,
    provider: AgentSkillProvider,
    desired_digest: &str,
) -> Result<()> {
    fs::create_dir_all(skills_root)?;
    let stage = unique_sibling(skills_root, "stage");
    fs::create_dir(&stage)?;
    let result = (|| {
        for asset in BUNDLED_ASSETS {
            let path = stage.join(asset.relative_path);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, asset.bytes)?;
        }
        let manifest = InstallManifest {
            schema: INSTALL_SCHEMA.to_string(),
            version: INSTALL_VERSION,
            provider,
            skill: TRAIL_LANES_SKILL.to_string(),
            content_digest: desired_digest.to_string(),
        };
        let mut manifest_bytes = serde_json::to_vec_pretty(&manifest)?;
        manifest_bytes.push(b'\n');
        fs::write(stage.join(INSTALL_MANIFEST), manifest_bytes)?;

        if !target.exists() {
            fs::rename(&stage, target)?;
            return Ok(());
        }

        let backup = unique_sibling(skills_root, "backup");
        fs::rename(target, &backup)?;
        if let Err(error) = fs::rename(&stage, target) {
            let _ = fs::rename(&backup, target);
            return Err(Error::Io(error));
        }
        fs::remove_dir_all(backup)?;
        Ok(())
    })();
    if result.is_err() && stage.exists() {
        let _ = fs::remove_dir_all(&stage);
    }
    result
}

fn bundled_digest() -> String {
    digest_entries(BUNDLED_ASSETS.iter().map(|asset| {
        (
            asset.relative_path.as_bytes().to_vec(),
            asset.bytes.to_vec(),
        )
    }))
}

fn installed_digest(root: &Path) -> Result<String> {
    let mut entries = Vec::new();
    let mut total_bytes = 0_u64;
    for entry in WalkDir::new(root).follow_links(false) {
        let entry = entry.map_err(|error| Error::InvalidPath {
            path: root.display().to_string(),
            reason: error.to_string(),
        })?;
        if entry.path() == root {
            continue;
        }
        if entry.file_type().is_symlink() {
            return Err(Error::InvalidPath {
                path: entry.path().display().to_string(),
                reason: "installed agent skills may not contain symlinks".to_string(),
            });
        }
        if !entry.file_type().is_file() || entry.file_name() == INSTALL_MANIFEST {
            continue;
        }
        let bytes = fs::read(entry.path())?;
        total_bytes = total_bytes.saturating_add(bytes.len() as u64);
        if total_bytes > MAX_INSTALLED_SKILL_BYTES {
            return Err(Error::InvalidInput(format!(
                "installed agent skill `{}` exceeds {} bytes",
                root.display(),
                MAX_INSTALLED_SKILL_BYTES
            )));
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| Error::InvalidPath {
                path: entry.path().display().to_string(),
                reason: "installed skill file escaped its root".to_string(),
            })?;
        let relative = relative.to_str().ok_or_else(|| Error::InvalidPath {
            path: entry.path().display().to_string(),
            reason: "installed skill file name is not valid Unicode".to_string(),
        })?;
        entries.push((relative.replace('\\', "/").into_bytes(), bytes));
    }
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(digest_entries(entries))
}

fn digest_entries(entries: impl IntoIterator<Item = (Vec<u8>, Vec<u8>)>) -> String {
    let mut digest = Sha256::new();
    for (path, bytes) in entries {
        digest.update((path.len() as u64).to_be_bytes());
        digest.update(path);
        digest.update((bytes.len() as u64).to_be_bytes());
        digest.update(bytes);
    }
    hex::encode(digest.finalize())
}

fn unique_sibling(parent: &Path, purpose: &str) -> PathBuf {
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    parent.join(format!(
        ".{TRAIL_LANES_SKILL}.{purpose}-{}-{sequence}",
        std::process::id()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_is_idempotent_and_protects_local_edits() {
        let home = tempfile::tempdir().unwrap();
        let config_root = home.path().join(".codex");
        let request = || AgentSkillInstallRequest {
            provider: AgentSkillProvider::Codex,
            config_root: &config_root,
            force: false,
            dry_run: false,
        };

        let created = install_agent_skills(request()).unwrap();
        assert_eq!(created.action, AgentSkillInstallAction::Create);
        assert!(created.path.join("SKILL.md").is_file());

        let repeated = install_agent_skills(request()).unwrap();
        assert_eq!(repeated.action, AgentSkillInstallAction::Noop);

        fs::write(created.path.join("SKILL.md"), "managed older version\n").unwrap();
        let manifest_path = created.path.join(INSTALL_MANIFEST);
        let mut manifest: InstallManifest =
            serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
        manifest.content_digest = installed_digest(&created.path).unwrap();
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&manifest).unwrap(),
        )
        .unwrap();
        let updated = install_agent_skills(request()).unwrap();
        assert_eq!(updated.action, AgentSkillInstallAction::Update);

        fs::write(created.path.join("SKILL.md"), "local edit\n").unwrap();
        let error = install_agent_skills(request()).unwrap_err();
        assert!(error.to_string().contains("contains local edits"));

        let forced = install_agent_skills(AgentSkillInstallRequest {
            force: true,
            ..request()
        })
        .unwrap();
        assert_eq!(forced.action, AgentSkillInstallAction::Update);
        assert!(fs::read_to_string(forced.path.join("SKILL.md"))
            .unwrap()
            .contains("name: trail-lanes"));
    }

    #[test]
    fn dry_run_does_not_create_provider_directories() {
        let home = tempfile::tempdir().unwrap();
        let config_root = home.path().join(".claude");
        let report = install_agent_skills(AgentSkillInstallRequest {
            provider: AgentSkillProvider::Claude,
            config_root: &config_root,
            force: false,
            dry_run: true,
        })
        .unwrap();
        assert_eq!(report.action, AgentSkillInstallAction::Create);
        assert!(report.dry_run);
        assert!(!config_root.exists());
    }

    #[test]
    fn force_can_replace_an_unmanaged_skill_directory() {
        let home = tempfile::tempdir().unwrap();
        let config_root = home.path().join(".claude");
        let skill = config_root.join("skills/trail-lanes");
        fs::create_dir_all(&skill).unwrap();
        fs::write(skill.join("SKILL.md"), "unmanaged\n").unwrap();

        let error = install_agent_skills(AgentSkillInstallRequest {
            provider: AgentSkillProvider::Claude,
            config_root: &config_root,
            force: false,
            dry_run: false,
        })
        .unwrap_err();
        assert!(error.to_string().contains("is not owned by Trail"));

        let report = install_agent_skills(AgentSkillInstallRequest {
            provider: AgentSkillProvider::Claude,
            config_root: &config_root,
            force: true,
            dry_run: false,
        })
        .unwrap();
        assert_eq!(report.action, AgentSkillInstallAction::Update);
        assert!(skill.join(INSTALL_MANIFEST).is_file());
    }
}
