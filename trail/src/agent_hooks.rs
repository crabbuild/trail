//! Provider registry and declarative native-agent hook contracts.
//!
//! Adapters describe discovery and translation only. Durable lifecycle policy remains
//! in Trail's shared capture coordinator.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::process::Command;

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

mod install;
mod parsing;

pub use install::{
    apply_agent_hook_install_plan, build_agent_hook_install_plan, inspect_agent_hook_installation,
    remove_agent_hook_installation, rollback_agent_hook_install_plan, AgentHookInstallAction,
    AgentHookInstallPlan, AgentHookInstallReport, AgentHookInstallRequest, AgentHookInstallScope,
    AgentHookInstallationRecord, AgentHookInstallationStatus,
};
pub use parsing::{parse_agent_hook_payload, AgentHookParseContext};

pub const AGENT_ADAPTER_MANIFEST_SCHEMA: &str = "trail.agent_adapter_manifest";
pub const AGENT_ADAPTER_MANIFEST_VERSION: u16 = 1;

/// Apply Trail's central secret-redaction policy before writing a fallback spool file.
pub fn redact_agent_hook_payload(value: serde_json::Value) -> serde_json::Value {
    crate::db::redact_sensitive_json(value)
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AgentAdapterDeploymentClass {
    JsonCommandConfig,
    ProjectPlugin,
    ProjectExtension,
    NativeOrCompatible,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProviderSupportLevel {
    Supported,
    Partial,
    Experimental,
    Unknown,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentHookEventBinding {
    pub native_event: String,
    pub normalized_events: Vec<String>,
    pub matcher: Option<String>,
    pub response_contract: String,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentProviderCapabilities {
    pub session_lifecycle: AgentProviderSupportLevel,
    pub turn_lifecycle: AgentProviderSupportLevel,
    pub messages: AgentProviderSupportLevel,
    pub tool_spans: AgentProviderSupportLevel,
    pub approvals: AgentProviderSupportLevel,
    pub subagents: AgentProviderSupportLevel,
    pub compaction: AgentProviderSupportLevel,
    pub usage: AgentProviderSupportLevel,
    pub native_transcript: AgentProviderSupportLevel,
    pub canonical_export: AgentProviderSupportLevel,
    pub context_injection: AgentProviderSupportLevel,
    pub acp: AgentProviderSupportLevel,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentAdapterManifest {
    pub schema: String,
    pub version: u16,
    pub provider: String,
    pub display_name: String,
    pub aliases: Vec<String>,
    pub adapter_version: String,
    pub deployment: AgentAdapterDeploymentClass,
    pub project_config_path: Option<String>,
    pub user_config_hint: Option<String>,
    pub provider_version_range: Option<String>,
    #[serde(default)]
    pub executable_candidates: Vec<String>,
    #[serde(default)]
    pub transcript_location_hints: Vec<String>,
    #[serde(default)]
    pub canonical_export_command: Option<Vec<String>>,
    pub support: AgentProviderSupportLevel,
    pub contract_source: Option<String>,
    pub events: Vec<AgentHookEventBinding>,
    pub capabilities: AgentProviderCapabilities,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct AgentProviderProbeReport {
    pub provider: String,
    pub executable: Option<PathBuf>,
    pub detected_version: Option<String>,
    pub support: AgentProviderSupportLevel,
    pub compatibility: AgentProviderSupportLevel,
    pub transcript_location_hints: Vec<String>,
    pub canonical_export_command: Option<Vec<String>>,
    pub diagnostics: Vec<String>,
}

impl AgentAdapterManifest {
    pub fn validate(&self) -> Result<()> {
        if self.schema != AGENT_ADAPTER_MANIFEST_SCHEMA
            || self.version != AGENT_ADAPTER_MANIFEST_VERSION
        {
            return Err(Error::InvalidInput(format!(
                "unsupported agent adapter manifest {} version {}",
                self.schema, self.version
            )));
        }
        validate_provider_name(&self.provider)?;
        if self.adapter_version.trim().is_empty() || self.display_name.trim().is_empty() {
            return Err(Error::InvalidInput(
                "agent adapter display name and version cannot be empty".to_string(),
            ));
        }
        let mut native_events = BTreeSet::new();
        for event in &self.events {
            if event.native_event.trim().is_empty() || event.normalized_events.is_empty() {
                return Err(Error::InvalidInput(format!(
                    "agent adapter `{}` contains an empty event binding",
                    self.provider
                )));
            }
            if !native_events.insert(event.native_event.as_str()) {
                return Err(Error::InvalidInput(format!(
                    "agent adapter `{}` repeats native event `{}`",
                    self.provider, event.native_event
                )));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct AgentProviderRegistry {
    manifests: BTreeMap<String, AgentAdapterManifest>,
    aliases: BTreeMap<String, String>,
}

impl AgentProviderRegistry {
    pub fn built_in() -> Result<Self> {
        Self::new(built_in_agent_adapter_manifests())
    }

    pub fn new(manifests: Vec<AgentAdapterManifest>) -> Result<Self> {
        let mut by_provider = BTreeMap::new();
        let mut aliases = BTreeMap::new();
        for manifest in manifests {
            manifest.validate()?;
            let provider = manifest.provider.clone();
            if by_provider
                .insert(provider.clone(), manifest.clone())
                .is_some()
            {
                return Err(Error::InvalidInput(format!(
                    "duplicate agent provider `{provider}`"
                )));
            }
            for alias in std::iter::once(provider.as_str())
                .chain(manifest.aliases.iter().map(String::as_str))
            {
                validate_provider_name(alias)?;
                if let Some(existing) = aliases.insert(alias.to_string(), provider.clone())
                    && existing != provider
                {
                    return Err(Error::InvalidInput(format!(
                            "agent provider alias `{alias}` belongs to both `{existing}` and `{provider}`"
                        )));
                }
            }
        }
        Ok(Self {
            manifests: by_provider,
            aliases,
        })
    }

    pub fn resolve(&self, name: &str) -> Result<&AgentAdapterManifest> {
        let normalized = name.trim().to_ascii_lowercase();
        let canonical = self
            .aliases
            .get(&normalized)
            .ok_or_else(|| Error::InvalidInput(format!("unknown agent hook provider `{name}`")))?;
        Ok(&self.manifests[canonical])
    }

    pub fn list(&self) -> Vec<&AgentAdapterManifest> {
        self.manifests.values().collect()
    }

    pub fn probe(&self, name: &str, execute_version: bool) -> Result<AgentProviderProbeReport> {
        let manifest = self.resolve(name)?;
        let executable = manifest
            .executable_candidates
            .iter()
            .find_map(|candidate| executable_in_path(candidate));
        let mut diagnostics = Vec::new();
        let detected_version = if execute_version {
            executable.as_ref().and_then(|executable| {
                match Command::new(executable).arg("--version").output() {
                    Ok(output) if output.status.success() => {
                        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
                        (!version.is_empty()).then_some(version)
                    }
                    Ok(output) => {
                        diagnostics.push(format!(
                            "`{} --version` exited with {}",
                            executable.display(),
                            output.status
                        ));
                        None
                    }
                    Err(error) => {
                        diagnostics.push(format!(
                            "could not execute `{} --version`: {error}",
                            executable.display()
                        ));
                        None
                    }
                }
            })
        } else {
            None
        };
        let compatibility = if executable.is_none() {
            AgentProviderSupportLevel::Unavailable
        } else if execute_version && detected_version.is_none() {
            AgentProviderSupportLevel::Unknown
        } else if execute_version {
            match (
                manifest.provider_version_range.as_deref(),
                detected_version.as_deref(),
            ) {
                (None, _) => {
                    diagnostics.push(
                        "the adapter has no pinned provider version range; treating the detected build as unknown until fixtures confirm it"
                            .to_string(),
                    );
                    AgentProviderSupportLevel::Unknown
                }
                (Some(range), Some(version)) => match version_satisfies_range(version, range) {
                    Some(true) => manifest.support,
                    Some(false) => {
                        diagnostics.push(format!(
                            "detected provider version `{version}` does not satisfy the verified adapter range `{range}`"
                        ));
                        AgentProviderSupportLevel::Unavailable
                    }
                    None => {
                        diagnostics.push(format!(
                            "could not compare detected provider version `{version}` with adapter range `{range}`"
                        ));
                        AgentProviderSupportLevel::Unknown
                    }
                },
                (Some(_), None) => AgentProviderSupportLevel::Unknown,
            }
        } else {
            manifest.support
        };
        if executable.is_none() {
            diagnostics.push(format!(
                "none of the provider executables were found on PATH: {}",
                manifest.executable_candidates.join(", ")
            ));
        }
        Ok(AgentProviderProbeReport {
            provider: manifest.provider.clone(),
            executable,
            detected_version,
            support: manifest.support,
            compatibility,
            transcript_location_hints: manifest.transcript_location_hints.clone(),
            canonical_export_command: manifest.canonical_export_command.clone(),
            diagnostics,
        })
    }
}

fn version_satisfies_range(version_output: &str, range: &str) -> Option<bool> {
    fn parse(value: &str) -> Option<(u64, u64, u64)> {
        let start = value.find(|character: char| character.is_ascii_digit())?;
        let token = value[start..]
            .split(|character: char| !(character.is_ascii_digit() || character == '.'))
            .next()?;
        let mut parts = token.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next().unwrap_or("0").parse().ok()?;
        let patch = parts.next().unwrap_or("0").parse().ok()?;
        Some((major, minor, patch))
    }

    let detected = parse(version_output)?;
    if let Some(minimum) = range.strip_prefix(">=") {
        return Some(detected >= parse(minimum.trim())?);
    }
    if let Some(maximum) = range.strip_prefix('<') {
        return Some(detected < parse(maximum.trim())?);
    }
    Some(detected == parse(range.trim())?)
}

fn executable_in_path(candidate: &str) -> Option<PathBuf> {
    let candidate_path = PathBuf::from(candidate);
    if candidate_path.components().count() > 1 {
        return candidate_path.is_file().then_some(candidate_path);
    }
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|directory| directory.join(candidate))
            .find(|path| path.is_file())
    })
}

fn validate_provider_name(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(Error::InvalidInput(format!(
            "invalid canonical agent provider name `{value}`"
        )));
    }
    Ok(())
}

fn binding(native: &str, normalized: &[&str]) -> AgentHookEventBinding {
    AgentHookEventBinding {
        native_event: native.to_string(),
        normalized_events: normalized
            .iter()
            .map(|value| (*value).to_string())
            .collect(),
        matcher: None,
        response_contract: "non-blocking-success".to_string(),
        notes: Vec::new(),
    }
}

fn capabilities(
    session: AgentProviderSupportLevel,
    turn: AgentProviderSupportLevel,
    tools: AgentProviderSupportLevel,
    transcript: AgentProviderSupportLevel,
    export: AgentProviderSupportLevel,
    acp: AgentProviderSupportLevel,
) -> AgentProviderCapabilities {
    AgentProviderCapabilities {
        session_lifecycle: session,
        turn_lifecycle: turn,
        messages: AgentProviderSupportLevel::Partial,
        tool_spans: tools,
        approvals: AgentProviderSupportLevel::Partial,
        subagents: AgentProviderSupportLevel::Partial,
        compaction: AgentProviderSupportLevel::Partial,
        usage: AgentProviderSupportLevel::Partial,
        native_transcript: transcript,
        canonical_export: export,
        context_injection: AgentProviderSupportLevel::Partial,
        acp,
    }
}

#[allow(
    clippy::too_many_arguments,
    reason = "mirrors the fixed provider manifest schema"
)]
fn manifest(
    provider: &str,
    display_name: &str,
    aliases: &[&str],
    deployment: AgentAdapterDeploymentClass,
    project_config_path: &str,
    support: AgentProviderSupportLevel,
    events: Vec<AgentHookEventBinding>,
    capabilities: AgentProviderCapabilities,
) -> AgentAdapterManifest {
    AgentAdapterManifest {
        schema: AGENT_ADAPTER_MANIFEST_SCHEMA.to_string(),
        version: AGENT_ADAPTER_MANIFEST_VERSION,
        provider: provider.to_string(),
        display_name: display_name.to_string(),
        aliases: aliases.iter().map(|alias| (*alias).to_string()).collect(),
        adapter_version: format!("trail/{provider}@2"),
        deployment,
        project_config_path: Some(project_config_path.to_string()),
        user_config_hint: None,
        provider_version_range: None,
        executable_candidates: match provider {
            "codex" => vec!["codex".to_string()],
            "claude-code" => vec!["claude".to_string()],
            "pi" => vec!["pi".to_string()],
            "opencode" => vec!["opencode".to_string()],
            "cursor" => vec!["agent".to_string(), "cursor-agent".to_string()],
            "gemini" => vec!["gemini".to_string()],
            "copilot" => vec!["copilot".to_string()],
            "grok" => vec!["grok".to_string()],
            "kiro" => vec!["kiro-cli".to_string()],
            _ => Vec::new(),
        },
        transcript_location_hints: Vec::new(),
        canonical_export_command: None,
        support,
        contract_source: None,
        events,
        capabilities,
        notes: Vec::new(),
    }
}

pub fn built_in_agent_adapter_manifests() -> Vec<AgentAdapterManifest> {
    use AgentAdapterDeploymentClass as Deployment;
    use AgentProviderSupportLevel as Support;

    let mut codex = manifest(
        "codex",
        "OpenAI Codex",
        &["openai-codex"],
        Deployment::JsonCommandConfig,
        ".codex/hooks.json",
        Support::Supported,
        vec![
            binding("SessionStart", &["session.started", "session.resumed"]),
            binding("UserPromptSubmit", &["turn.started", "message.user"]),
            binding("PreToolUse", &["tool.started"]),
            binding("PermissionRequest", &["approval.requested"]),
            binding("PostToolUse", &["tool.completed", "tool.failed"]),
            binding("PreCompact", &["compaction.started"]),
            binding("PostCompact", &["compaction.completed"]),
            binding("SubagentStart", &["subagent.started"]),
            binding("SubagentStop", &["subagent.completed", "subagent.failed"]),
            binding("Stop", &["turn.completed", "turn.failed", "turn.cancelled"]),
        ],
        capabilities(
            Support::Partial,
            Support::Supported,
            Support::Supported,
            Support::Partial,
            Support::Unavailable,
            Support::Supported,
        ),
    );
    codex.contract_source = Some("https://learn.chatgpt.com/docs/hooks".to_string());
    codex.notes = vec![
        "Project hooks require the .codex configuration layer and exact hook definition to be trusted."
            .to_string(),
        "Codex currently exposes SessionStart but no native SessionEnd hook; Stop closes turns."
            .to_string(),
        "Transcript paths are optional and the transcript format is not a stable hook interface."
            .to_string(),
    ];
    codex.transcript_location_hints =
        vec!["hook payload transcript_path when supplied".to_string()];

    let mut claude = manifest(
        "claude-code",
        "Anthropic Claude Code",
        &["claude"],
        Deployment::JsonCommandConfig,
        ".claude/settings.json",
        Support::Supported,
        vec![
            binding("SessionStart", &["session.started", "session.resumed"]),
            binding("SessionEnd", &["session.ended"]),
            binding("UserPromptSubmit", &["turn.started", "message.user"]),
            binding("PreToolUse", &["tool.started"]),
            binding("PostToolUse", &["tool.completed"]),
            binding("PostToolUseFailure", &["tool.failed"]),
            binding("PermissionRequest", &["approval.requested"]),
            binding("PermissionDenied", &["approval.decided"]),
            binding("Stop", &["turn.completed"]),
            binding("StopFailure", &["turn.failed"]),
            binding("SubagentStart", &["subagent.started"]),
            binding("SubagentStop", &["subagent.completed", "subagent.failed"]),
            binding("PreCompact", &["compaction.started"]),
            binding("PostCompact", &["compaction.completed"]),
        ],
        capabilities(
            Support::Supported,
            Support::Supported,
            Support::Supported,
            Support::Supported,
            Support::Unavailable,
            Support::Partial,
        ),
    );
    claude.contract_source = Some("https://code.claude.com/docs/en/hooks".to_string());
    claude.transcript_location_hints =
        vec!["hook payload transcript_path under Claude's project session directory".to_string()];

    let mut pi = manifest(
        "pi",
        "Pi",
        &["pi-coding-agent"],
        Deployment::ProjectExtension,
        ".pi/extensions/trail/index.ts",
        Support::Partial,
        vec![
            binding("session_start", &["session.started", "session.resumed"]),
            binding("session_shutdown", &["session.ended"]),
            binding("before_agent_start", &["turn.started", "message.user"]),
            binding("agent_start", &["session.updated"]),
            binding(
                "agent_end",
                &["turn.completed", "turn.failed", "turn.cancelled"],
            ),
            binding("message_update", &["message.assistant.delta"]),
            binding("tool_execution_start", &["tool.started"]),
            binding("tool_execution_end", &["tool.completed", "tool.failed"]),
            binding("session_before_compact", &["compaction.started"]),
            binding("session_compact", &["compaction.completed"]),
        ],
        capabilities(
            Support::Supported,
            Support::Supported,
            Support::Supported,
            Support::Supported,
            Support::Unavailable,
            Support::Unknown,
        ),
    );
    pi.contract_source = Some("https://pi.dev/docs/latest/extensions".to_string());
    pi.transcript_location_hints = vec![
        "extension context sessionManager.getSessionFile() under ~/.pi/agent/sessions".to_string(),
    ];

    let mut opencode = manifest(
        "opencode",
        "OpenCode",
        &["open-code"],
        Deployment::ProjectPlugin,
        ".opencode/plugins/trail.ts",
        Support::Partial,
        vec![
            binding("session.created", &["session.started"]),
            binding("session.updated", &["session.updated"]),
            binding("session.idle", &["turn.completed"]),
            binding("session.deleted", &["session.ended"]),
            binding("chat.message", &["turn.started", "message.user"]),
            binding("message.updated", &["message.assistant.delta"]),
            binding("message.part.updated", &["message.assistant.delta"]),
            binding("tool.execute.before", &["tool.started"]),
            binding("tool.execute.after", &["tool.completed", "tool.failed"]),
            binding("permission.asked", &["approval.requested"]),
            binding("permission.replied", &["approval.decided"]),
            binding("session.compacted", &["compaction.completed"]),
            binding("session.diff", &["workspace.diff"]),
            binding("file.edited", &["workspace.file_changed"]),
            binding("todo.updated", &["plan.updated"]),
            binding("command.executed", &["diagnostic"]),
            binding("session.error", &["diagnostic"]),
        ],
        capabilities(
            Support::Supported,
            Support::Supported,
            Support::Supported,
            Support::Partial,
            Support::Supported,
            Support::Partial,
        ),
    );
    opencode.contract_source = Some("https://opencode.ai/docs/plugins/".to_string());
    opencode.canonical_export_command = Some(vec![
        "opencode".to_string(),
        "export".to_string(),
        "{session_id}".to_string(),
    ]);

    let mut cursor = manifest(
        "cursor",
        "Cursor",
        &["cursor-agent"],
        Deployment::JsonCommandConfig,
        ".cursor/hooks.json",
        Support::Partial,
        vec![
            binding("sessionStart", &["session.started", "session.resumed"]),
            binding("sessionEnd", &["session.ended"]),
            binding("beforeSubmitPrompt", &["turn.started", "message.user"]),
            binding("preToolUse", &["tool.started"]),
            binding("postToolUse", &["tool.completed", "tool.failed"]),
            binding("postToolUseFailure", &["tool.failed"]),
            binding("afterAgentResponse", &["message.assistant.completed"]),
            binding("afterAgentThought", &["diagnostic"]),
            binding("stop", &["turn.completed", "turn.failed", "turn.cancelled"]),
            binding("subagentStart", &["subagent.started"]),
            binding("subagentStop", &["subagent.completed", "subagent.failed"]),
            binding("preCompact", &["compaction.started"]),
        ],
        capabilities(
            Support::Partial,
            Support::Supported,
            Support::Partial,
            Support::Partial,
            Support::Unavailable,
            Support::Supported,
        ),
    );
    cursor.contract_source = Some("https://cursor.com/docs/hooks".to_string());
    cursor.transcript_location_hints = vec![
        "hook payload transcript_path when exposed by the installed Cursor surface".to_string(),
    ];

    let mut gemini = manifest(
        "gemini",
        "Google Gemini CLI",
        &["gemini-cli"],
        Deployment::JsonCommandConfig,
        ".gemini/settings.json",
        Support::Partial,
        vec![
            binding("SessionStart", &["session.started", "session.resumed"]),
            binding("SessionEnd", &["session.ended"]),
            binding("BeforeAgent", &["turn.started", "message.user"]),
            binding(
                "AfterAgent",
                &["turn.completed", "turn.failed", "turn.cancelled"],
            ),
            binding("BeforeModel", &["model.updated"]),
            binding("AfterModel", &["usage.updated"]),
            binding("BeforeToolSelection", &["diagnostic"]),
            binding("BeforeTool", &["tool.started"]),
            binding("AfterTool", &["tool.completed", "tool.failed"]),
            binding("PreCompress", &["compaction.started"]),
            binding("Notification", &["diagnostic"]),
        ],
        capabilities(
            Support::Supported,
            Support::Supported,
            Support::Supported,
            Support::Supported,
            Support::Unavailable,
            Support::Unknown,
        ),
    );
    gemini.contract_source = Some("https://geminicli.com/docs/hooks/reference/".to_string());
    gemini.transcript_location_hints =
        vec!["hook payload transcript_path for the native JSON transcript".to_string()];

    let mut copilot = manifest(
        "copilot",
        "GitHub Copilot CLI",
        &["copilot-cli", "github-copilot"],
        Deployment::JsonCommandConfig,
        ".github/hooks/trail.json",
        Support::Partial,
        vec![
            binding("sessionStart", &["session.started", "session.resumed"]),
            binding("sessionEnd", &["session.ended"]),
            binding("userPromptSubmitted", &["turn.started", "message.user"]),
            binding("preToolUse", &["tool.started"]),
            binding("postToolUse", &["tool.completed", "tool.failed"]),
            binding("postToolUseFailure", &["tool.failed"]),
            binding("permissionRequest", &["approval.requested"]),
            binding("notification", &["diagnostic"]),
            binding("errorOccurred", &["diagnostic"]),
            binding("preCompact", &["compaction.started"]),
            binding(
                "agentStop",
                &["turn.completed", "turn.failed", "turn.cancelled"],
            ),
            binding("subagentStart", &["subagent.started"]),
            binding("subagentStop", &["subagent.completed", "subagent.failed"]),
        ],
        capabilities(
            Support::Supported,
            Support::Supported,
            Support::Supported,
            Support::Supported,
            Support::Unavailable,
            Support::Unknown,
        ),
    );
    copilot.contract_source =
        Some("https://docs.github.com/en/copilot/reference/hooks-reference".to_string());
    copilot.transcript_location_hints =
        vec!["validated Copilot session-state events.jsonl".to_string()];

    let mut grok = manifest(
        "grok",
        "xAI Grok Build",
        &["grok-build"],
        Deployment::NativeOrCompatible,
        ".grok/hooks/trail.json",
        Support::Supported,
        vec![
            binding("SessionStart", &["session.started", "session.resumed"]),
            binding("SessionEnd", &["session.ended"]),
            binding("UserPromptSubmit", &["turn.started", "message.user"]),
            binding("PreToolUse", &["tool.started"]),
            binding("PostToolUse", &["tool.completed"]),
            binding("PostToolUseFailure", &["tool.failed"]),
            binding("PermissionDenied", &["approval.decided"]),
            binding("Notification", &["diagnostic"]),
            binding("Stop", &["turn.completed"]),
            binding("StopFailure", &["turn.failed"]),
            binding("SubagentStart", &["subagent.started"]),
            binding("SubagentStop", &["subagent.completed", "subagent.failed"]),
            binding("PreCompact", &["compaction.started"]),
            binding("PostCompact", &["compaction.completed"]),
        ],
        capabilities(
            Support::Supported,
            Support::Supported,
            Support::Supported,
            Support::Supported,
            Support::Partial,
            Support::Supported,
        ),
    );
    grok.contract_source = Some("https://docs.x.ai/build/features/hooks".to_string());
    grok.transcript_location_hints = vec!["~/.grok/sessions".to_string()];
    grok.canonical_export_command = Some(vec![
        "grok".to_string(),
        "export".to_string(),
        "{session_id}".to_string(),
    ]);
    grok.notes = vec![
        "Project hook files require explicit /hooks-trust or a trusted launch.".to_string(),
        "Grok also reads Claude Code and Cursor hook configuration, but Trail prefers an owned .grok hook file."
            .to_string(),
        "ACP through `grok agent stdio` is the stable high-fidelity path when available."
            .to_string(),
    ];

    let mut kiro = manifest(
        "kiro",
        "Kiro",
        &["kiro-cli"],
        Deployment::NativeOrCompatible,
        ".kiro/hooks/trail.json",
        Support::Experimental,
        vec![
            binding("SessionStart", &["session.started", "session.resumed"]),
            binding("UserPromptSubmit", &["turn.started", "message.user"]),
            binding("PreToolUse", &["tool.started"]),
            binding("PostToolUse", &["tool.completed", "tool.failed"]),
            binding(
                "Stop",
                &[
                    "message.assistant.completed",
                    "turn.completed",
                    "turn.failed",
                    "turn.cancelled",
                ],
            ),
            binding("PreTaskExec", &["subagent.started"]),
            binding("PostTaskExec", &["subagent.completed", "subagent.failed"]),
        ],
        capabilities(
            Support::Partial,
            Support::Supported,
            Support::Supported,
            Support::Unavailable,
            Support::Unavailable,
            Support::Unknown,
        ),
    );
    kiro.adapter_version = "trail/kiro@3".to_string();
    kiro.contract_source = Some("https://kiro.dev/docs/hooks/".to_string());
    kiro.provider_version_range = Some(">=2.8.0".to_string());
    kiro.notes = vec![
        "Trail installs Kiro's versioned standalone hook format under .kiro/hooks for the IDE and the CLI v3 engine. Kiro CLI package 2.8.0 and newer expose that engine with `kiro-cli --v3`; the default v2 engine continues to use embedded custom-agent hooks."
            .to_string(),
        "Kiro currently exposes SessionStart but no standalone SessionEnd trigger; Stop closes turns."
            .to_string(),
    ];

    vec![
        codex, claude, pi, opencode, cursor, gemini, copilot, grok, kiro,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn built_in_registry_has_nine_unique_providers_and_stable_aliases() {
        let registry = AgentProviderRegistry::built_in().unwrap();
        assert_eq!(registry.list().len(), 9);
        assert_eq!(registry.resolve("claude").unwrap().provider, "claude-code");
        assert_eq!(registry.resolve("gemini-cli").unwrap().provider, "gemini");
        assert_eq!(registry.resolve("grok-build").unwrap().provider, "grok");
        assert_eq!(registry.resolve("kiro-cli").unwrap().provider, "kiro");
        assert!(registry.resolve("unknown").is_err());
    }

    #[test]
    fn codex_manifest_matches_current_documented_command_hook_set() {
        let registry = AgentProviderRegistry::built_in().unwrap();
        let codex = registry.resolve("codex").unwrap();
        let events = codex
            .events
            .iter()
            .map(|event| event.native_event.as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            events,
            BTreeSet::from([
                "PermissionRequest",
                "PostCompact",
                "PostToolUse",
                "PreCompact",
                "PreToolUse",
                "SessionStart",
                "Stop",
                "SubagentStart",
                "SubagentStop",
                "UserPromptSubmit",
            ])
        );
        assert!(!events.contains("SessionEnd"));
        assert_eq!(
            codex.contract_source.as_deref(),
            Some("https://learn.chatgpt.com/docs/hooks")
        );
    }

    #[test]
    fn every_binding_targets_normalized_or_provider_namespaced_events() {
        for manifest in built_in_agent_adapter_manifests() {
            manifest.validate().unwrap();
            for event in manifest.events {
                for normalized in event.normalized_events {
                    let event_type = crate::AgentLifecycleEventType::new(normalized.clone());
                    assert!(
                        event_type.kind() != crate::AgentLifecycleEventKind::Unknown
                            || normalized.starts_with(&format!("provider.{}.", manifest.provider)),
                        "{} -> {}",
                        event.native_event,
                        normalized
                    );
                }
            }
        }
    }

    #[test]
    fn provider_version_ranges_are_compared_against_version_command_output() {
        assert_eq!(
            version_satisfies_range("kiro-cli 2.7.0", ">=2.8.0"),
            Some(false)
        );
        assert_eq!(
            version_satisfies_range("kiro-cli 2.8.0", ">=2.8.0"),
            Some(true)
        );
        assert_eq!(
            version_satisfies_range("kiro-cli 2.12.1", ">=2.8.0"),
            Some(true)
        );
        assert_eq!(version_satisfies_range("unknown", ">=2.8.0"), None);
    }
}
