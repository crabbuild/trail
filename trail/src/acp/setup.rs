use std::env;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use super::acp_provider_profile_with_registry;
use crate::{Error, Result};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct AcpSetupReport {
    pub transport: String,
    pub provider: String,
    pub editor: String,
    pub command: Vec<String>,
    pub snippet: String,
    pub config_path: Option<PathBuf>,
    pub action: String,
    pub before_digest: Option<String>,
    pub after_digest: Option<String>,
    pub applied: bool,
    pub warnings: Vec<String>,
    #[serde(skip)]
    before_bytes: Option<Vec<u8>>,
    #[serde(skip)]
    desired_bytes: Option<Vec<u8>>,
}

pub fn build_acp_setup_plan(
    workspace_root: &Path,
    db_dir: &Path,
    provider: &str,
    editor: &str,
) -> Result<AcpSetupReport> {
    let workspace_root = workspace_root.canonicalize()?;
    let executable = env::current_exe()?.canonicalize()?;
    let profile = acp_provider_profile_with_registry(provider, Some(db_dir))?;
    if !profile.supports_acp {
        return Err(Error::InvalidInput(format!(
            "provider `{}` does not support ACP",
            profile.agent
        )));
    }
    let mut warnings = profile.notes;
    if !matches!(editor, "generic" | "vscode" | "zed") {
        warnings.push(format!(
            "editor `{editor}` has no exact Trail adapter; returning a generic entry without changing editor settings"
        ));
    }
    let command = vec![
        executable.to_string_lossy().to_string(),
        "--workspace".to_string(),
        workspace_root.to_string_lossy().to_string(),
        "agent".to_string(),
        "acp".to_string(),
        "run".to_string(),
        profile.agent.clone(),
    ];
    let snippet = editor_snippet(editor, &profile.agent, &command);
    let Some(config_path) = editor_config_path(editor)? else {
        return Ok(AcpSetupReport {
            transport: "acp".to_string(),
            provider: profile.agent,
            editor: editor.to_string(),
            command,
            snippet,
            config_path: None,
            action: "print".to_string(),
            before_digest: None,
            after_digest: None,
            applied: false,
            warnings,
            before_bytes: None,
            desired_bytes: None,
        });
    };
    let before_bytes = match fs::read(&config_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    let desired_bytes = merge_zed_settings(before_bytes.as_deref(), &profile.agent, &command)?;
    let action = match before_bytes.as_deref() {
        None => "create",
        Some(before) if before == desired_bytes => "noop",
        Some(_) => "update",
    };
    Ok(AcpSetupReport {
        transport: "acp".to_string(),
        provider: profile.agent,
        editor: editor.to_string(),
        command,
        snippet,
        config_path: Some(config_path),
        action: action.to_string(),
        before_digest: before_bytes.as_deref().map(content_digest),
        after_digest: Some(content_digest(&desired_bytes)),
        applied: false,
        warnings,
        before_bytes,
        desired_bytes: Some(desired_bytes),
    })
}

pub fn apply_acp_setup_plan(mut plan: AcpSetupReport, apply: bool) -> Result<AcpSetupReport> {
    if !apply || plan.action == "print" {
        return Ok(plan);
    }
    let config_path = plan.config_path.as_ref().ok_or_else(|| {
        Error::InvalidInput("ACP setup plan has no writable editor target".to_string())
    })?;
    let desired = plan.desired_bytes.as_deref().ok_or_else(|| {
        Error::InvalidInput("ACP setup plan has no desired editor configuration".to_string())
    })?;
    let current = match fs::read(config_path) {
        Ok(bytes) => Some(bytes),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(error.into()),
    };
    if current.as_deref().map(content_digest) != plan.before_digest {
        return Err(Error::Conflict(format!(
            "ACP editor config `{}` changed after planning; build a new plan",
            config_path.display()
        )));
    }
    if plan.action != "noop" {
        if let Some(before) = plan.before_bytes.as_deref() {
            write_backup(&plan.provider, &plan.editor, before)?;
        }
        atomic_write(config_path, desired)?;
    }
    plan.applied = true;
    Ok(plan)
}

fn editor_config_path(editor: &str) -> Result<Option<PathBuf>> {
    if editor != "zed" {
        return Ok(None);
    }
    let home = env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
        Error::InvalidInput("cannot resolve home directory for Zed setup".to_string())
    })?;
    #[cfg(target_os = "macos")]
    return Ok(Some(
        home.join("Library/Application Support/Zed/settings.json"),
    ));
    #[cfg(target_os = "windows")]
    return Ok(Some(
        env::var_os("APPDATA")
            .map(PathBuf::from)
            .unwrap_or(home)
            .join("Zed/settings.json"),
    ));
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    Ok(Some(
        env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".config"))
            .join("zed/settings.json"),
    ))
}

fn merge_zed_settings(
    existing: Option<&[u8]>,
    provider: &str,
    command: &[String],
) -> Result<Vec<u8>> {
    let mut root = match existing {
        None => Map::new(),
        Some(bytes) => serde_json::from_slice::<Value>(bytes)?
            .as_object()
            .cloned()
            .ok_or_else(|| {
                Error::Conflict("Zed settings must contain a JSON object".to_string())
            })?,
    };
    let servers = root
        .entry("agent_servers".to_string())
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(|| Error::Conflict("Zed agent_servers must be a JSON object".to_string()))?;
    servers.insert(
        format!("trail-{provider}"),
        serde_json::json!({
            "type": "custom",
            "command": command.first().cloned().unwrap_or_default(),
            "args": command.iter().skip(1).cloned().collect::<Vec<_>>()
        }),
    );
    let mut bytes = serde_json::to_vec_pretty(&Value::Object(root))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn editor_snippet(editor: &str, provider: &str, command: &[String]) -> String {
    if editor == "zed" {
        return serde_json::to_string_pretty(&serde_json::json!({
            "agent_servers": {
                (format!("trail-{provider}")): {
                    "type": "custom",
                    "command": command.first().cloned().unwrap_or_default(),
                    "args": command.iter().skip(1).cloned().collect::<Vec<_>>()
                }
            }
        }))
        .unwrap_or_else(|_| "{}".to_string());
    }
    if editor == "vscode" {
        return serde_json::to_string_pretty(&serde_json::json!({
            (format!("Trail {provider}")): {
                "command": command.first().cloned().unwrap_or_default(),
                "args": command.iter().skip(1).cloned().collect::<Vec<_>>(),
                "env": {}
            }
        }))
        .unwrap_or_else(|_| "{}".to_string());
    }
    format!("ACP command:\n{}", shell_join(command))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path.parent().ok_or_else(|| Error::InvalidPath {
        path: path.display().to_string(),
        reason: "ACP editor config has no parent directory".to_string(),
    })?;
    fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".trail-acp-{}.tmp", std::process::id()));
    fs::write(&temp, bytes)?;
    if let Ok(metadata) = fs::metadata(path) {
        fs::set_permissions(&temp, metadata.permissions())?;
    } else {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&temp, fs::Permissions::from_mode(0o600))?;
        }
    }
    if let Err(error) = fs::rename(&temp, path) {
        let _ = fs::remove_file(&temp);
        return Err(error.into());
    }
    Ok(())
}

fn write_backup(provider: &str, editor: &str, bytes: &[u8]) -> Result<()> {
    let root = application_state_dir()?.join("trail/backups/agent-acp");
    fs::create_dir_all(&root)?;
    let digest = content_digest(bytes);
    atomic_write(
        &root.join(format!("{editor}-{provider}-{}.json", &digest[..12])),
        bytes,
    )
}

fn application_state_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("XDG_STATE_HOME") {
        return Ok(PathBuf::from(path));
    }
    #[cfg(target_os = "macos")]
    {
        let home = env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            Error::InvalidInput("cannot resolve application state directory".to_string())
        })?;
        Ok(home.join("Library/Application Support"))
    }
    #[cfg(target_os = "windows")]
    {
        return env::var_os("LOCALAPPDATA")
            .map(PathBuf::from)
            .ok_or_else(|| Error::InvalidInput("cannot resolve LOCALAPPDATA".to_string()));
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        let home = env::var_os("HOME").map(PathBuf::from).ok_or_else(|| {
            Error::InvalidInput("cannot resolve application state directory".to_string())
        })?;
        Ok(home.join(".local/state"))
    }
}

fn content_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn shell_join(parts: &[String]) -> String {
    parts
        .iter()
        .map(|part| {
            if part.chars().all(|ch| {
                ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '/' | '.' | '@' | ':')
            }) {
                part.clone()
            } else {
                format!("'{}'", part.replace('\'', "'\\''"))
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
