use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use super::{AgentAdapterDeploymentClass, AgentAdapterManifest, AgentProviderRegistry};
use crate::{Error, Result};

const MAX_CONFIG_BYTES: u64 = 2 * 1024 * 1024;
const OWNERSHIP_PREFIX: &str = "trail ";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHookInstallScope {
    Project,
    User,
}

impl AgentHookInstallScope {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Project => "project",
            Self::User => "user",
        }
    }
}

#[derive(Clone, Debug)]
pub struct AgentHookInstallRequest<'a> {
    pub registry: &'a AgentProviderRegistry,
    pub provider: &'a str,
    pub workspace_id: &'a str,
    pub workspace_root: &'a Path,
    pub home_dir: Option<&'a Path>,
    pub scope: AgentHookInstallScope,
    pub force: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHookInstallAction {
    Create,
    Update,
    Noop,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentHookInstallPlan {
    pub installation_id: String,
    pub provider: String,
    pub scope: AgentHookInstallScope,
    pub adapter_version: String,
    pub provider_version_range: Option<String>,
    pub config_path: PathBuf,
    pub action: AgentHookInstallAction,
    pub before_digest: Option<String>,
    pub after_digest: String,
    pub manifest_digest: String,
    pub ownership_inventory: Vec<String>,
    #[serde(skip)]
    pub before_bytes: Option<Vec<u8>>,
    pub desired_bytes: Vec<u8>,
    pub owned_file: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentHookInstallReport {
    pub installation_id: String,
    pub provider: String,
    pub scope: AgentHookInstallScope,
    pub config_path: PathBuf,
    pub action: AgentHookInstallAction,
    pub before_digest: Option<String>,
    pub after_digest: String,
    pub ownership_inventory: Vec<String>,
    pub dry_run: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentHookInstallationRecord {
    pub installation_id: String,
    pub workspace_id: String,
    pub provider: String,
    pub scope: AgentHookInstallScope,
    pub config_path: PathBuf,
    pub lane_id: Option<String>,
    pub manifest_digest: String,
    pub ownership_inventory: Vec<String>,
    pub config_before_digest: Option<String>,
    pub config_after_digest: String,
    pub adapter_version: String,
    pub provider_version_range: Option<String>,
    pub detected_provider_version: Option<String>,
    pub capability_status: String,
    pub status: String,
    pub installed_at: i64,
    pub verified_at: Option<i64>,
    pub last_receipt_at: Option<i64>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentHookInstallationStatus {
    Missing,
    Installed,
    Drifted,
    Foreign,
    Malformed,
}

pub fn build_agent_hook_install_plan(
    request: AgentHookInstallRequest<'_>,
) -> Result<AgentHookInstallPlan> {
    let manifest = request.registry.resolve(request.provider)?;
    let installation_id =
        stable_installation_id(request.workspace_id, &manifest.provider, request.scope);
    let (base, relative) = target_for(
        manifest,
        request.scope,
        request.workspace_root,
        request.home_dir,
    )?;
    validate_relative_target(&relative)?;
    let base = canonicalize_or_create_base(base)?;
    let workspace_root =
        request
            .workspace_root
            .canonicalize()
            .map_err(|error| Error::InvalidPath {
                path: request.workspace_root.display().to_string(),
                reason: format!("cannot canonicalize hook workspace root: {error}"),
            })?;
    let target = base.join(&relative);
    validate_target_path(&base, &target)?;
    let before = read_bounded_optional(&target)?;
    let before_digest = before.as_deref().map(content_digest);
    let (desired_bytes, inventory, owned_file) = desired_asset(
        manifest,
        &installation_id,
        &workspace_root,
        before.as_deref(),
        request.force,
    )?;
    validate_provider_asset(manifest, &desired_bytes)?;
    let after_digest = content_digest(&desired_bytes);
    let manifest_digest = manifest_digest(manifest)?;
    let action = match before.as_deref() {
        None => AgentHookInstallAction::Create,
        Some(existing) if existing == desired_bytes => AgentHookInstallAction::Noop,
        Some(_) => AgentHookInstallAction::Update,
    };
    Ok(AgentHookInstallPlan {
        installation_id,
        provider: manifest.provider.clone(),
        scope: request.scope,
        adapter_version: manifest.adapter_version.clone(),
        provider_version_range: manifest.provider_version_range.clone(),
        config_path: target,
        action,
        before_digest,
        after_digest,
        manifest_digest,
        ownership_inventory: inventory,
        before_bytes: before,
        desired_bytes,
        owned_file,
    })
}

pub fn rollback_agent_hook_install_plan(plan: &AgentHookInstallPlan) -> Result<()> {
    let _target_lock = acquire_target_lock(&plan.config_path)?;
    let current = read_bounded_optional(&plan.config_path)?;
    if current.as_deref().map(content_digest).as_deref() != Some(plan.after_digest.as_str()) {
        return Err(Error::Conflict(format!(
            "cannot roll back `{}` because it changed after Trail installed it",
            plan.config_path.display()
        )));
    }
    if let Some(before) = plan.before_bytes.as_deref() {
        atomic_write(&plan.config_path, before, 0o600)
    } else {
        fs::remove_file(&plan.config_path)?;
        sync_parent(&plan.config_path)
    }
}

pub fn apply_agent_hook_install_plan(
    plan: &AgentHookInstallPlan,
    dry_run: bool,
) -> Result<AgentHookInstallReport> {
    if !dry_run && plan.action != AgentHookInstallAction::Noop {
        let _target_lock = acquire_target_lock(&plan.config_path)?;
        let current = read_bounded_optional(&plan.config_path)?;
        if current.as_deref().map(content_digest) != plan.before_digest {
            return Err(Error::Conflict(format!(
                "agent hook config `{}` changed after planning; build a new install plan",
                plan.config_path.display()
            )));
        }
        atomic_write(&plan.config_path, &plan.desired_bytes, 0o600)?;
    }
    Ok(AgentHookInstallReport {
        installation_id: plan.installation_id.clone(),
        provider: plan.provider.clone(),
        scope: plan.scope,
        config_path: plan.config_path.clone(),
        action: plan.action.clone(),
        before_digest: plan.before_digest.clone(),
        after_digest: plan.after_digest.clone(),
        ownership_inventory: plan.ownership_inventory.clone(),
        dry_run,
    })
}

pub fn inspect_agent_hook_installation(
    plan: &AgentHookInstallPlan,
) -> Result<AgentHookInstallationStatus> {
    let Some(bytes) = read_bounded_optional(&plan.config_path)? else {
        return Ok(AgentHookInstallationStatus::Missing);
    };
    if bytes == plan.desired_bytes {
        return Ok(AgentHookInstallationStatus::Installed);
    }
    let manifest = AgentProviderRegistry::built_in()?
        .resolve(&plan.provider)?
        .clone();
    if validate_provider_asset(&manifest, &bytes).is_err() {
        return Ok(AgentHookInstallationStatus::Malformed);
    }
    if plan.owned_file {
        let text = String::from_utf8_lossy(&bytes);
        return Ok(
            if text.contains(&format!("installation={}", plan.installation_id))
                || owned_json_installation(&bytes).as_deref() == Some(plan.installation_id.as_str())
            {
                AgentHookInstallationStatus::Drifted
            } else {
                AgentHookInstallationStatus::Foreign
            },
        );
    }
    let root: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) => return Ok(AgentHookInstallationStatus::Malformed),
    };
    Ok(
        if json_contains_installation(&root, &plan.installation_id) {
            AgentHookInstallationStatus::Drifted
        } else {
            AgentHookInstallationStatus::Missing
        },
    )
}

pub fn remove_agent_hook_installation(
    plan: &AgentHookInstallPlan,
    expected_after_digest: &str,
    dry_run: bool,
) -> Result<AgentHookInstallReport> {
    let _target_lock = if dry_run {
        None
    } else {
        Some(acquire_target_lock(&plan.config_path)?)
    };
    let Some(bytes) = read_bounded_optional(&plan.config_path)? else {
        return Ok(AgentHookInstallReport {
            installation_id: plan.installation_id.clone(),
            provider: plan.provider.clone(),
            scope: plan.scope,
            config_path: plan.config_path.clone(),
            action: AgentHookInstallAction::Noop,
            before_digest: None,
            after_digest: content_digest(&[]),
            ownership_inventory: plan.ownership_inventory.clone(),
            dry_run,
        });
    };
    let current_digest = content_digest(&bytes);
    let removed = if plan.owned_file {
        if current_digest != expected_after_digest {
            return Err(Error::Conflict(format!(
                "refusing to remove modified Trail-owned hook file `{}`: expected {}, found {}",
                plan.config_path.display(),
                expected_after_digest,
                current_digest
            )));
        }
        None
    } else {
        Some(remove_owned_json_entries(&bytes, &plan.installation_id)?)
    };
    let after_digest = removed
        .as_deref()
        .map(content_digest)
        .unwrap_or_else(|| content_digest(&[]));
    if !dry_run {
        if let Some(content) = removed.as_deref() {
            atomic_write(&plan.config_path, content, 0o600)?;
        } else {
            fs::remove_file(&plan.config_path)?;
            sync_parent(&plan.config_path)?;
        }
    }
    Ok(AgentHookInstallReport {
        installation_id: plan.installation_id.clone(),
        provider: plan.provider.clone(),
        scope: plan.scope,
        config_path: plan.config_path.clone(),
        action: AgentHookInstallAction::Update,
        before_digest: Some(current_digest),
        after_digest,
        ownership_inventory: plan.ownership_inventory.clone(),
        dry_run,
    })
}

fn stable_installation_id(
    workspace_id: &str,
    provider: &str,
    scope: AgentHookInstallScope,
) -> String {
    let digest = Sha256::digest(
        format!(
            "trail-agent-hook-v1:{workspace_id}:{provider}:{}",
            scope.as_str()
        )
        .as_bytes(),
    );
    format!("hook_{}", hex(&digest[..16]))
}

fn target_for(
    manifest: &AgentAdapterManifest,
    scope: AgentHookInstallScope,
    workspace_root: &Path,
    home: Option<&Path>,
) -> Result<(PathBuf, PathBuf)> {
    let project = manifest.project_config_path.as_deref().ok_or_else(|| {
        Error::InvalidInput(format!("{} has no install target", manifest.provider))
    })?;
    match scope {
        AgentHookInstallScope::Project => {
            Ok((workspace_root.to_path_buf(), PathBuf::from(project)))
        }
        AgentHookInstallScope::User => {
            let home = home.ok_or_else(|| {
                Error::InvalidInput("cannot resolve user home for hook installation".to_string())
            })?;
            let relative = match manifest.provider.as_str() {
                "codex" => ".codex/hooks.json",
                "claude-code" => ".claude/settings.json",
                "pi" => ".pi/agent/extensions/trail/index.ts",
                "opencode" => ".config/opencode/plugins/trail.ts",
                "cursor" => ".cursor/hooks.json",
                "gemini" => ".gemini/settings.json",
                "copilot" => ".copilot/hooks/trail.json",
                "grok" => ".grok/hooks/trail.json",
                "kiro" => ".kiro/hooks/trail.json",
                other => {
                    return Err(Error::InvalidInput(format!(
                        "provider `{other}` does not declare a user hook target"
                    )))
                }
            };
            Ok((home.to_path_buf(), PathBuf::from(relative)))
        }
    }
}

fn canonicalize_or_create_base(base: PathBuf) -> Result<PathBuf> {
    fs::create_dir_all(&base)?;
    Ok(base.canonicalize()?)
}

fn validate_relative_target(path: &Path) -> Result<()> {
    if path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(Error::InvalidPath {
            path: path.display().to_string(),
            reason: "agent hook target must be a relative path without parent traversal"
                .to_string(),
        });
    }
    Ok(())
}

fn validate_target_path(base: &Path, target: &Path) -> Result<()> {
    if !target.starts_with(base) {
        return Err(Error::InvalidPath {
            path: target.display().to_string(),
            reason: "agent hook target escaped its install scope".to_string(),
        });
    }
    let relative = target.strip_prefix(base).map_err(|_| Error::InvalidPath {
        path: target.display().to_string(),
        reason: "agent hook target escaped its install scope".to_string(),
    })?;
    let mut cursor = base.to_path_buf();
    for component in relative.components() {
        cursor.push(component);
        match fs::symlink_metadata(&cursor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(Error::InvalidPath {
                    path: cursor.display().to_string(),
                    reason: "agent hook installation refuses symlinked path components".to_string(),
                })
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn desired_asset(
    manifest: &AgentAdapterManifest,
    installation_id: &str,
    workspace_root: &Path,
    existing: Option<&[u8]>,
    force: bool,
) -> Result<(Vec<u8>, Vec<String>, bool)> {
    match manifest.deployment {
        AgentAdapterDeploymentClass::ProjectExtension
        | AgentAdapterDeploymentClass::ProjectPlugin => {
            let content = generated_typescript(manifest, installation_id, workspace_root);
            refuse_foreign_owned_file(existing, installation_id, force)?;
            Ok((content.into_bytes(), vec!["owned-file".to_string()], true))
        }
        AgentAdapterDeploymentClass::NativeOrCompatible
            if matches!(manifest.provider.as_str(), "grok" | "kiro") =>
        {
            let value = generated_owned_json(manifest, installation_id, workspace_root);
            let content = json_bytes(&value)?;
            refuse_foreign_owned_json(existing, installation_id, force)?;
            Ok((content, vec!["owned-file".to_string()], true))
        }
        AgentAdapterDeploymentClass::JsonCommandConfig if manifest.provider == "copilot" => {
            let value = generated_owned_json(manifest, installation_id, workspace_root);
            let content = json_bytes(&value)?;
            refuse_foreign_owned_json(existing, installation_id, force)?;
            Ok((content, vec!["owned-file".to_string()], true))
        }
        AgentAdapterDeploymentClass::JsonCommandConfig
        | AgentAdapterDeploymentClass::NativeOrCompatible => {
            let (content, inventory) =
                merge_json_hooks(manifest, installation_id, workspace_root, existing)?;
            Ok((content, inventory, false))
        }
    }
}

fn merge_json_hooks(
    manifest: &AgentAdapterManifest,
    installation_id: &str,
    workspace_root: &Path,
    existing: Option<&[u8]>,
) -> Result<(Vec<u8>, Vec<String>)> {
    let mut root = parse_json_object(existing)?;
    if manifest.provider == "cursor" {
        match root.get("version") {
            Some(Value::Number(version)) if version.as_u64() == Some(1) => {}
            Some(_) => {
                return Err(Error::Conflict(
                    "Cursor hook config `version` must be the number 1".to_string(),
                ))
            }
            None => {
                root.insert("version".to_string(), Value::from(1));
            }
        }
    }
    let hooks = root
        .entry("hooks".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    let hooks = hooks.as_object_mut().ok_or_else(|| {
        Error::Conflict("provider config `hooks` field is not a JSON object".to_string())
    })?;
    remove_owned_from_hook_map(hooks, installation_id)?;
    let mut inventory = Vec::new();
    for binding in &manifest.events {
        let entries = hooks
            .entry(binding.native_event.clone())
            .or_insert_with(|| Value::Array(Vec::new()));
        let entries = entries.as_array_mut().ok_or_else(|| {
            Error::Conflict(format!(
                "provider hook event `{}` is not a JSON array",
                binding.native_event
            ))
        })?;
        let command = hook_command(
            &manifest.provider,
            &binding.native_event,
            installation_id,
            workspace_root,
        );
        let entry = hook_config_entry(manifest.provider.as_str(), binding, command);
        entries.push(entry);
        inventory.push(format!("hooks.{}:{installation_id}", binding.native_event));
    }
    Ok((json_bytes(&Value::Object(root))?, inventory))
}

fn generated_owned_json(
    manifest: &AgentAdapterManifest,
    installation_id: &str,
    workspace_root: &Path,
) -> Value {
    if manifest.provider == "kiro" {
        let hooks = manifest
            .events
            .iter()
            .map(|binding| {
                json!({
                    "name": format!("trail-{}", kiro_hook_name(&binding.native_event)),
                    "description": "Record this Kiro lifecycle event in Trail",
                    "trigger": binding.native_event,
                    "action": {
                        "type": "command",
                        "command": hook_command(
                            &manifest.provider,
                            &binding.native_event,
                            installation_id,
                            workspace_root,
                        )
                    },
                    "timeout": hook_timeout_seconds(&binding.native_event),
                    "enabled": true
                })
            })
            .collect::<Vec<_>>();
        return json!({"version": "v1", "hooks": hooks});
    }

    let mut hooks = Map::new();
    for binding in &manifest.events {
        let command = hook_command(
            &manifest.provider,
            &binding.native_event,
            installation_id,
            workspace_root,
        );
        let entry = if manifest.provider == "copilot" {
            json!({
                "type": "command",
                "bash": command,
                "powershell": hook_command_powershell(
                    &manifest.provider,
                    &binding.native_event,
                    installation_id,
                    workspace_root,
                ),
                "timeoutSec": hook_timeout_seconds(&binding.native_event)
            })
        } else {
            hook_config_entry(manifest.provider.as_str(), binding, command)
        };
        hooks.insert(binding.native_event.clone(), Value::Array(vec![entry]));
    }
    let mut root = Map::new();
    if manifest.provider == "copilot" {
        root.insert("version".to_string(), Value::from(1));
    }
    root.insert("hooks".to_string(), Value::Object(hooks));
    Value::Object(root)
}

/// Render a provider hook group without serializing absent optional fields as JSON null.
/// Claude Code treats an omitted matcher as match-all; `matcher: null` is not a valid
/// matcher value and causes version-dependent configuration drift.
fn hook_config_entry(
    provider: &str,
    binding: &crate::agent_hooks::AgentHookEventBinding,
    command: String,
) -> Value {
    if provider == "cursor" {
        let mut entry = Map::new();
        entry.insert("command".to_string(), Value::String(command));
        entry.insert(
            "timeout".to_string(),
            Value::from(hook_timeout_seconds(&binding.native_event)),
        );
        if let Some(matcher) = binding.matcher.as_ref() {
            entry.insert("matcher".to_string(), Value::String(matcher.clone()));
        }
        return Value::Object(entry);
    }
    let mut entry = Map::new();
    if let Some(matcher) = binding.matcher.as_ref() {
        entry.insert("matcher".to_string(), Value::String(matcher.clone()));
    }
    entry.insert(
        "hooks".to_string(),
        json!([{
            "type": "command",
            "command": command,
            "timeout": hook_timeout_value(provider, &binding.native_event)
        }]),
    );
    Value::Object(entry)
}

fn generated_typescript(
    manifest: &AgentAdapterManifest,
    installation_id: &str,
    workspace_root: &Path,
) -> String {
    let header = format!(
        "// Generated by Trail. installation={installation_id} adapter={}\n// Manage with: trail agent hooks remove {}\n",
        manifest.adapter_version, manifest.provider
    );
    let bindings = manifest
        .events
        .iter()
        .map(|binding| {
            format!(
                "  [{} , {}],",
                serde_json::to_string(&binding.native_event).expect("string serialization"),
                serde_json::to_string(&hook_command(
                    &manifest.provider,
                    &binding.native_event,
                    installation_id,
                    workspace_root,
                ))
                .expect("string serialization")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    if manifest.provider == "pi" {
        format!(
            "{header}import type {{ ExtensionAPI }} from \"@earendil-works/pi-coding-agent\";\nimport {{ spawn }} from \"node:child_process\";\n\nconst hooks = new Map<string, string>([\n{bindings}\n]);\nfunction send(command: string, payload: unknown): Promise<void> {{\n  return new Promise((resolve) => {{\n    try {{\n      const child = spawn(command, {{ shell: true, stdio: [\"pipe\", \"ignore\", \"ignore\"] }});\n      const timer = setTimeout(() => {{ child.kill(); resolve(); }}, 30000);\n      child.once(\"error\", () => {{ clearTimeout(timer); resolve(); }});\n      child.once(\"close\", () => {{ clearTimeout(timer); resolve(); }});\n      child.stdin.end(JSON.stringify(payload));\n    }} catch {{ resolve(); }}\n  }});\n}}\nexport default function trail(pi: ExtensionAPI): void {{\n  for (const [event, command] of hooks) {{\n    (pi.on as any)(event, async (payload: Record<string, unknown>, ctx: any) => {{\n      const sessionFile = ctx?.sessionManager?.getSessionFile?.();\n      await send(command, {{ ...payload, session_id: (payload as any).session_id ?? sessionFile, transcript_path: sessionFile, cwd: (payload as any).cwd ?? ctx?.cwd }});\n    }});\n  }}\n}}\n"
        )
    } else {
        format!(
            "{header}import type {{ Plugin }} from \"@opencode-ai/plugin\";\nimport {{ spawn }} from \"node:child_process\";\n\nconst hooks = new Map<string, string>([\n{bindings}\n]);\nfunction send(event: string, payload: unknown): Promise<void> {{\n  const command = hooks.get(event);\n  if (!command) return Promise.resolve();\n  return new Promise((resolve) => {{\n    try {{\n      const child = spawn(command, {{ shell: true, stdio: [\"pipe\", \"ignore\", \"ignore\"] }});\n      const timer = setTimeout(() => {{ child.kill(); resolve(); }}, 30000);\n      child.once(\"error\", () => {{ clearTimeout(timer); resolve(); }});\n      child.once(\"close\", () => {{ clearTimeout(timer); resolve(); }});\n      child.stdin.end(JSON.stringify(payload));\n    }} catch {{ resolve(); }}\n  }});\n}}\nexport const TrailPlugin: Plugin = async (ctx) => ({{\n  event: async (input: {{ event: {{ type: string }} }}) => send(input.event.type, {{ ...input.event, cwd: ctx.directory }}),\n  \"chat.message\": async (input: unknown, output: unknown) => send(\"chat.message\", {{ input, output, cwd: ctx.directory }}),\n  \"tool.execute.before\": async (input: unknown, output: unknown) => send(\"tool.execute.before\", {{ input, output, cwd: ctx.directory }}),\n  \"tool.execute.after\": async (input: unknown, output: unknown) => send(\"tool.execute.after\", {{ input, output, cwd: ctx.directory }}),\n}});\n"
        )
    }
}

fn hook_command(
    provider: &str,
    event: &str,
    installation_id: &str,
    workspace_root: &Path,
) -> String {
    format!(
        "trail --workspace {} agent hook receive {} {} --installation {}",
        shell_quote(&workspace_root.to_string_lossy()),
        shell_quote(provider),
        shell_quote(event),
        shell_quote(installation_id)
    )
}

fn shell_quote(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn hook_command_powershell(
    provider: &str,
    event: &str,
    installation_id: &str,
    workspace_root: &Path,
) -> String {
    fn quote(value: &str) -> String {
        format!("'{}'", value.replace('\'', "''"))
    }
    format!(
        "trail --workspace {} agent hook receive {} {} --installation {}",
        quote(&workspace_root.to_string_lossy()),
        quote(provider),
        quote(event),
        quote(installation_id),
    )
}

fn hook_timeout_seconds(event: &str) -> u64 {
    if event.to_ascii_lowercase().contains("stop")
        || event.to_ascii_lowercase().contains("end")
        || event.to_ascii_lowercase().contains("shutdown")
    {
        60
    } else {
        30
    }
}

fn hook_timeout_value(provider: &str, event: &str) -> u64 {
    let seconds = hook_timeout_seconds(event);
    if provider == "gemini" {
        seconds.saturating_mul(1_000)
    } else {
        seconds
    }
}

fn kiro_hook_name(event: &str) -> String {
    event
        .chars()
        .enumerate()
        .flat_map(|(index, character)| {
            if character.is_ascii_uppercase() && index > 0 {
                vec!['-', character.to_ascii_lowercase()]
            } else {
                vec![character.to_ascii_lowercase()]
            }
        })
        .collect()
}

fn parse_json_object(existing: Option<&[u8]>) -> Result<Map<String, Value>> {
    let Some(bytes) = existing else {
        return Ok(Map::new());
    };
    let value: Value = serde_json::from_slice(bytes).map_err(|error| {
        Error::Conflict(format!("provider hook config is not valid JSON: {error}"))
    })?;
    value.as_object().cloned().ok_or_else(|| {
        Error::Conflict("provider hook config root is not a JSON object".to_string())
    })
}

fn remove_owned_json_entries(bytes: &[u8], installation_id: &str) -> Result<Vec<u8>> {
    let mut root = parse_json_object(Some(bytes))?;
    let Some(hooks) = root.get_mut("hooks") else {
        return Err(Error::Conflict(format!(
            "Trail installation `{installation_id}` is absent from provider config"
        )));
    };
    let hooks = hooks.as_object_mut().ok_or_else(|| {
        Error::Conflict("provider config `hooks` field is not a JSON object".to_string())
    })?;
    let removed = remove_owned_from_hook_map(hooks, installation_id)?;
    if removed == 0 {
        return Err(Error::Conflict(format!(
            "Trail installation `{installation_id}` is absent or its ownership marker drifted"
        )));
    }
    if hooks.is_empty() {
        root.remove("hooks");
    }
    json_bytes(&Value::Object(root))
}

fn remove_owned_from_hook_map(
    hooks: &mut Map<String, Value>,
    installation_id: &str,
) -> Result<usize> {
    let marker = format!("--installation {installation_id}");
    let mut removed = 0;
    for (event, value) in hooks.iter_mut() {
        let entries = value.as_array_mut().ok_or_else(|| {
            Error::Conflict(format!("provider hook event `{event}` is not a JSON array"))
        })?;
        entries.retain_mut(|entry| {
            if let Some(inner) = entry.get_mut("hooks") {
                let Some(inner) = inner.as_array_mut() else {
                    return true;
                };
                let before = inner.len();
                inner.retain(|hook| !entry_matches_marker(hook, &marker));
                removed += before - inner.len();
                !inner.is_empty()
            } else if entry_matches_marker(entry, &marker) {
                removed += 1;
                false
            } else {
                true
            }
        });
    }
    hooks.retain(|_, value| value.as_array().is_some_and(|entries| !entries.is_empty()));
    Ok(removed)
}

fn entry_matches_marker(entry: &Value, marker: &str) -> bool {
    ["command", "bash", "powershell"].iter().any(|key| {
        entry
            .get(*key)
            .and_then(Value::as_str)
            .is_some_and(|command| {
                command.starts_with(OWNERSHIP_PREFIX)
                    && command.contains(" agent hook receive ")
                    && command.contains(marker)
            })
    })
}

fn json_contains_installation(value: &Value, installation_id: &str) -> bool {
    match value {
        Value::String(value) => value.contains(&format!("--installation {installation_id}")),
        Value::Array(values) => values
            .iter()
            .any(|value| json_contains_installation(value, installation_id)),
        Value::Object(values) => values
            .values()
            .any(|value| json_contains_installation(value, installation_id)),
        _ => false,
    }
}

fn refuse_foreign_owned_file(
    existing: Option<&[u8]>,
    installation_id: &str,
    force: bool,
) -> Result<()> {
    if let Some(bytes) = existing {
        let text = String::from_utf8_lossy(bytes);
        if !text.contains(&format!(
            "Generated by Trail. installation={installation_id}"
        )) && !force
        {
            return Err(Error::Conflict(
                "refusing to overwrite a foreign provider plugin/extension file; pass --force only after reviewing it"
                    .to_string(),
            ));
        }
    }
    Ok(())
}

fn refuse_foreign_owned_json(
    existing: Option<&[u8]>,
    installation_id: &str,
    force: bool,
) -> Result<()> {
    if let Some(bytes) = existing {
        let owned = owned_json_installation(bytes).is_some_and(|value| value == installation_id);
        if !owned && !force {
            return Err(Error::Conflict(
                "refusing to overwrite a foreign provider hook file; pass --force only after reviewing it"
                    .to_string(),
            ));
        }
    }
    Ok(())
}

fn owned_json_installation(bytes: &[u8]) -> Option<String> {
    let value = serde_json::from_slice::<Value>(bytes).ok()?;
    value
        .get("trailOwnership")
        .and_then(|value| value.get("installation"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| find_installation_marker(&value))
}

fn find_installation_marker(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => value
            .split_whitespace()
            .collect::<Vec<_>>()
            .windows(2)
            .find_map(|pair| {
                (pair[0] == "--installation" && pair[1].starts_with("hook_"))
                    .then(|| pair[1].trim_matches(['\'', '"']).to_string())
            }),
        Value::Array(values) => values.iter().find_map(find_installation_marker),
        Value::Object(values) => values.values().find_map(find_installation_marker),
        _ => None,
    }
}

fn validate_provider_asset(manifest: &AgentAdapterManifest, bytes: &[u8]) -> Result<()> {
    if matches!(
        manifest.deployment,
        AgentAdapterDeploymentClass::ProjectExtension | AgentAdapterDeploymentClass::ProjectPlugin
    ) {
        std::str::from_utf8(bytes).map_err(|error| {
            Error::Conflict(format!(
                "{} hook extension is not UTF-8: {error}",
                manifest.provider
            ))
        })?;
        return Ok(());
    }

    let root: Value = serde_json::from_slice(bytes).map_err(|error| {
        Error::Conflict(format!(
            "{} hook config is not valid JSON: {error}",
            manifest.provider
        ))
    })?;
    let root = root.as_object().ok_or_else(|| {
        Error::Conflict(format!(
            "{} hook config root must be a JSON object",
            manifest.provider
        ))
    })?;

    match manifest.provider.as_str() {
        "kiro" => validate_kiro_hook_config(root),
        "cursor" => {
            require_version(root, Value::from(1), "Cursor")?;
            validate_flat_hook_map(root, "Cursor", &["command", "prompt"])
        }
        "copilot" => {
            require_version(root, Value::from(1), "Copilot")?;
            validate_flat_hook_map(
                root,
                "Copilot",
                &["bash", "powershell", "command", "url", "prompt"],
            )
        }
        _ => validate_nested_hook_map(root, &manifest.display_name),
    }
}

fn require_version(root: &Map<String, Value>, expected: Value, provider: &str) -> Result<()> {
    if root.get("version") != Some(&expected) {
        return Err(Error::Conflict(format!(
            "{provider} hook config has an unsupported or missing `version`"
        )));
    }
    Ok(())
}

fn validate_kiro_hook_config(root: &Map<String, Value>) -> Result<()> {
    require_version(root, Value::String("v1".to_string()), "Kiro")?;
    let hooks = root
        .get("hooks")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Conflict("Kiro hook config `hooks` must be an array".to_string()))?;
    for hook in hooks {
        let hook = hook
            .as_object()
            .ok_or_else(|| Error::Conflict("Kiro hook definitions must be objects".to_string()))?;
        if hook.get("name").and_then(Value::as_str).is_none()
            || hook.get("trigger").and_then(Value::as_str).is_none()
            || hook.get("action").and_then(Value::as_object).is_none()
        {
            return Err(Error::Conflict(
                "Kiro hooks require string `name`, string `trigger`, and object `action` fields"
                    .to_string(),
            ));
        }
        let action = hook["action"].as_object().expect("checked above");
        if action.get("type").and_then(Value::as_str) != Some("command")
            || action.get("command").and_then(Value::as_str).is_none()
        {
            return Err(Error::Conflict(
                "Trail Kiro hooks require command actions".to_string(),
            ));
        }
    }
    Ok(())
}

fn validate_flat_hook_map(
    root: &Map<String, Value>,
    provider: &str,
    command_fields: &[&str],
) -> Result<()> {
    let Some(hooks_value) = root.get("hooks") else {
        return Ok(());
    };
    let hooks = hooks_value.as_object().ok_or_else(|| {
        Error::Conflict(format!("{provider} hook config `hooks` must be an object"))
    })?;
    for (event, entries) in hooks {
        let entries = entries.as_array().ok_or_else(|| {
            Error::Conflict(format!("{provider} hook event `{event}` must be an array"))
        })?;
        for entry in entries {
            let entry = entry.as_object().ok_or_else(|| {
                Error::Conflict(format!(
                    "{provider} hook event `{event}` contains a non-object entry"
                ))
            })?;
            if !command_fields
                .iter()
                .any(|field| entry.get(*field).and_then(Value::as_str).is_some())
            {
                return Err(Error::Conflict(format!(
                    "{provider} hook event `{event}` has no supported command, URL, or prompt field"
                )));
            }
        }
    }
    Ok(())
}

fn validate_nested_hook_map(root: &Map<String, Value>, provider: &str) -> Result<()> {
    let Some(hooks_value) = root.get("hooks") else {
        return Ok(());
    };
    let hooks = hooks_value.as_object().ok_or_else(|| {
        Error::Conflict(format!("{provider} hook config `hooks` must be an object"))
    })?;
    for (event, groups) in hooks {
        let groups = groups.as_array().ok_or_else(|| {
            Error::Conflict(format!("{provider} hook event `{event}` must be an array"))
        })?;
        for group in groups {
            let handlers = group
                .get("hooks")
                .and_then(Value::as_array)
                .ok_or_else(|| {
                    Error::Conflict(format!(
                        "{provider} hook event `{event}` groups require a `hooks` array"
                    ))
                })?;
            for handler in handlers {
                let handler = handler.as_object().ok_or_else(|| {
                    Error::Conflict(format!(
                        "{provider} hook event `{event}` contains a non-object handler"
                    ))
                })?;
                if handler.get("type").and_then(Value::as_str).is_none() {
                    return Err(Error::Conflict(format!(
                        "{provider} hook event `{event}` handler is missing `type`"
                    )));
                }
            }
        }
    }
    Ok(())
}

fn read_bounded_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(Error::InvalidPath {
            path: path.display().to_string(),
            reason: "agent hook target must be a regular non-symlink file".to_string(),
        });
    }
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(Error::InvalidInput(format!(
            "agent hook config `{}` exceeds {} bytes",
            path.display(),
            MAX_CONFIG_BYTES
        )));
    }
    Ok(Some(fs::read(path)?))
}

fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().ok_or_else(|| Error::InvalidPath {
        path: path.display().to_string(),
        reason: "agent hook target has no parent directory".to_string(),
    })?;
    fs::create_dir_all(parent)?;
    validate_target_path(&parent.canonicalize()?, path)?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::InvalidPath {
            path: path.display().to_string(),
            reason: "agent hook target filename is not valid UTF-8".to_string(),
        })?;
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    let temp = parent.join(format!(".{name}.trail-{nonce}.tmp"));
    let result = (|| -> Result<()> {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(mode);
        }
        let mut file = options.open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temp, path)?;
        sync_parent(path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temp);
    }
    result
}

struct TargetLock {
    path: PathBuf,
}

impl Drop for TargetLock {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn acquire_target_lock(target: &Path) -> Result<TargetLock> {
    let parent = target.parent().ok_or_else(|| Error::InvalidPath {
        path: target.display().to_string(),
        reason: "agent hook target has no parent directory".to_string(),
    })?;
    fs::create_dir_all(parent)?;
    let name = target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::InvalidPath {
            path: target.display().to_string(),
            reason: "agent hook target filename is not valid UTF-8".to_string(),
        })?;
    let path = parent.join(format!(".{name}.trail-install.lock"));
    for attempt in 0..2 {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut file) => {
                writeln!(
                    file,
                    "pid={} created_nanos={}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .map(|duration| duration.as_nanos())
                        .unwrap_or_default()
                )?;
                file.sync_all()?;
                return Ok(TargetLock { path });
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists && attempt == 0 => {
                let metadata = fs::symlink_metadata(&path)?;
                if metadata.file_type().is_symlink() || !metadata.is_file() {
                    return Err(Error::InvalidPath {
                        path: path.display().to_string(),
                        reason: "agent hook install lock is not a regular file".to_string(),
                    });
                }
                let stale = metadata
                    .modified()
                    .ok()
                    .and_then(|modified| modified.elapsed().ok())
                    .is_some_and(|elapsed| elapsed.as_secs() > 600);
                if stale {
                    fs::remove_file(&path)?;
                    continue;
                }
                return Err(Error::WorkspaceLocked(format!(
                    "agent hook config `{}` is being modified by another installer",
                    target.display()
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(Error::WorkspaceLocked(format!(
                    "agent hook config `{}` is being modified by another installer",
                    target.display()
                )));
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(Error::WorkspaceLocked(format!(
        "could not acquire agent hook install lock for `{}`",
        target.display()
    )))
}

fn sync_parent(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let parent = path.parent().ok_or_else(|| Error::InvalidPath {
            path: path.display().to_string(),
            reason: "agent hook target has no parent directory".to_string(),
        })?;
        OpenOptions::new().read(true).open(parent)?.sync_all()?;
    }
    Ok(())
}

fn manifest_digest(manifest: &AgentAdapterManifest) -> Result<String> {
    Ok(content_digest(&serde_json::to_vec(manifest)?))
}

fn content_digest(bytes: &[u8]) -> String {
    format!("sha256:{}", hex(&Sha256::digest(bytes)))
}

fn json_bytes(value: &Value) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec_pretty(value)?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request<'a>(
        registry: &'a AgentProviderRegistry,
        provider: &'a str,
        root: &'a Path,
    ) -> AgentHookInstallRequest<'a> {
        AgentHookInstallRequest {
            registry,
            provider,
            workspace_id: "workspace-test",
            workspace_root: root,
            home_dir: Some(root),
            scope: AgentHookInstallScope::Project,
            force: false,
        }
    }

    #[test]
    fn json_merge_preserves_unrelated_fields_and_hooks() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".codex")).unwrap();
        fs::write(
            temp.path().join(".codex/hooks.json"),
            br#"{"$schema":"kept","hooks":{"Stop":[{"hooks":[{"type":"command","command":"user stop"}]}]}}"#,
        )
        .unwrap();
        let registry = AgentProviderRegistry::built_in().unwrap();
        let plan = build_agent_hook_install_plan(request(&registry, "codex", temp.path())).unwrap();
        apply_agent_hook_install_plan(&plan, false).unwrap();
        let value: Value = serde_json::from_slice(&fs::read(&plan.config_path).unwrap()).unwrap();
        assert_eq!(value["$schema"], "kept");
        assert_eq!(value["hooks"]["Stop"].as_array().unwrap().len(), 2);
        assert_eq!(
            inspect_agent_hook_installation(&plan).unwrap(),
            AgentHookInstallationStatus::Installed
        );
    }

    #[test]
    fn install_is_idempotent_and_uninstall_removes_only_owned_entries() {
        let temp = tempfile::tempdir().unwrap();
        let registry = AgentProviderRegistry::built_in().unwrap();
        let plan =
            build_agent_hook_install_plan(request(&registry, "claude", temp.path())).unwrap();
        apply_agent_hook_install_plan(&plan, false).unwrap();
        let second =
            build_agent_hook_install_plan(request(&registry, "claude-code", temp.path())).unwrap();
        assert_eq!(second.action, AgentHookInstallAction::Noop);
        remove_agent_hook_installation(&second, &plan.after_digest, false).unwrap();
        let value: Value = serde_json::from_slice(&fs::read(&plan.config_path).unwrap()).unwrap();
        assert!(value.get("hooks").is_none());
    }

    #[test]
    fn claude_hook_groups_omit_unset_matchers() {
        let temp = tempfile::tempdir().unwrap();
        let registry = AgentProviderRegistry::built_in().unwrap();
        let plan =
            build_agent_hook_install_plan(request(&registry, "claude-code", temp.path())).unwrap();
        apply_agent_hook_install_plan(&plan, false).unwrap();
        let value: Value = serde_json::from_slice(&fs::read(&plan.config_path).unwrap()).unwrap();
        for groups in value["hooks"].as_object().unwrap().values() {
            for group in groups.as_array().unwrap() {
                assert!(group.get("matcher").is_none());
                assert!(group["hooks"].is_array());
            }
        }
    }

    #[test]
    fn provider_specific_json_contracts_have_valid_versions_and_timeout_units() {
        let registry = AgentProviderRegistry::built_in().unwrap();

        let cursor_temp = tempfile::tempdir().unwrap();
        let cursor =
            build_agent_hook_install_plan(request(&registry, "cursor", cursor_temp.path()))
                .unwrap();
        let cursor_json: Value = serde_json::from_slice(&cursor.desired_bytes).unwrap();
        assert_eq!(cursor_json["version"], 1);
        assert!(cursor_json["hooks"]["preToolUse"][0]["timeout"]
            .as_u64()
            .is_some_and(|timeout| timeout >= 30));

        let gemini_temp = tempfile::tempdir().unwrap();
        let gemini =
            build_agent_hook_install_plan(request(&registry, "gemini", gemini_temp.path()))
                .unwrap();
        let gemini_json: Value = serde_json::from_slice(&gemini.desired_bytes).unwrap();
        assert_eq!(
            gemini_json["hooks"]["BeforeTool"][0]["hooks"][0]["timeout"],
            30_000
        );

        let copilot_temp = tempfile::tempdir().unwrap();
        let copilot =
            build_agent_hook_install_plan(request(&registry, "copilot", copilot_temp.path()))
                .unwrap();
        let copilot_json: Value = serde_json::from_slice(&copilot.desired_bytes).unwrap();
        assert_eq!(copilot_json["version"], 1);
        assert!(copilot_json.get("trailOwnership").is_none());
    }

    #[test]
    fn kiro_uses_the_versioned_standalone_hook_contract() {
        let temp = tempfile::tempdir().unwrap();
        let registry = AgentProviderRegistry::built_in().unwrap();
        let plan = build_agent_hook_install_plan(request(&registry, "kiro", temp.path())).unwrap();
        let value: Value = serde_json::from_slice(&plan.desired_bytes).unwrap();
        assert_eq!(value["version"], "v1");
        assert_eq!(plan.provider_version_range.as_deref(), Some(">=2.8.0"));
        assert!(value.get("trailOwnership").is_none());
        let hooks = value["hooks"].as_array().unwrap();
        assert_eq!(hooks.len(), registry.resolve("kiro").unwrap().events.len());
        assert!(hooks.iter().all(|hook| {
            hook["name"].as_str().is_some()
                && hook["trigger"].as_str().is_some()
                && hook["action"]["type"] == "command"
                && hook["action"]["command"]
                    .as_str()
                    .is_some_and(|command| command.contains("--installation hook_"))
        }));
    }

    #[test]
    fn strict_json_validation_reports_malformed_drift() {
        let temp = tempfile::tempdir().unwrap();
        let registry = AgentProviderRegistry::built_in().unwrap();
        let plan =
            build_agent_hook_install_plan(request(&registry, "cursor", temp.path())).unwrap();
        apply_agent_hook_install_plan(&plan, false).unwrap();
        fs::write(&plan.config_path, br#"{"version":"one","hooks":{}}"#).unwrap();
        assert_eq!(
            inspect_agent_hook_installation(&plan).unwrap(),
            AgentHookInstallationStatus::Malformed
        );
    }

    #[test]
    fn malformed_hook_shape_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".gemini")).unwrap();
        fs::write(
            temp.path().join(".gemini/settings.json"),
            br#"{"hooks":{"SessionStart":"not-an-array"}}"#,
        )
        .unwrap();
        let registry = AgentProviderRegistry::built_in().unwrap();
        let error =
            build_agent_hook_install_plan(request(&registry, "gemini", temp.path())).unwrap_err();
        assert!(error.to_string().contains("not a JSON array"));
    }

    #[test]
    fn owned_plugin_refuses_foreign_file_and_drifted_removal() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join(".opencode/plugins")).unwrap();
        let path = temp.path().join(".opencode/plugins/trail.ts");
        fs::write(&path, "foreign").unwrap();
        let registry = AgentProviderRegistry::built_in().unwrap();
        let error =
            build_agent_hook_install_plan(request(&registry, "opencode", temp.path())).unwrap_err();
        assert!(error.to_string().contains("foreign"));

        fs::remove_file(&path).unwrap();
        let plan =
            build_agent_hook_install_plan(request(&registry, "opencode", temp.path())).unwrap();
        apply_agent_hook_install_plan(&plan, false).unwrap();
        fs::write(
            &path,
            format!("{}// drift", String::from_utf8_lossy(&plan.desired_bytes)),
        )
        .unwrap();
        let error = remove_agent_hook_installation(&plan, &plan.after_digest, false).unwrap_err();
        assert!(error.to_string().contains("modified"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_config_component_is_rejected() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), temp.path().join(".codex")).unwrap();
        let registry = AgentProviderRegistry::built_in().unwrap();
        let error =
            build_agent_hook_install_plan(request(&registry, "codex", temp.path())).unwrap_err();
        assert!(error.to_string().contains("symlink"));
    }

    #[test]
    fn all_nine_providers_render_project_and_user_assets() {
        let registry = AgentProviderRegistry::built_in().unwrap();
        for manifest in registry.list() {
            for scope in [AgentHookInstallScope::Project, AgentHookInstallScope::User] {
                let temp = tempfile::tempdir().unwrap();
                let mut req = request(&registry, &manifest.provider, temp.path());
                req.scope = scope;
                let plan = build_agent_hook_install_plan(req).unwrap();
                assert!(
                    !plan.desired_bytes.is_empty(),
                    "{} {scope:?}",
                    manifest.provider
                );
                apply_agent_hook_install_plan(&plan, false).unwrap();
                let mut second = request(&registry, &manifest.provider, temp.path());
                second.scope = scope;
                let second = build_agent_hook_install_plan(second).unwrap();
                assert_eq!(
                    second.action,
                    AgentHookInstallAction::Noop,
                    "{} {scope:?}",
                    manifest.provider
                );
            }
        }
    }
}
