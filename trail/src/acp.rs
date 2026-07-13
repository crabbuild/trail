use std::collections::{BTreeMap, HashMap, HashSet};
use std::env;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::model::*;
use crate::{Error, PatchDocument, PatchEdit, Result, Trail};

mod protocol;
mod registry;
mod schema;
mod setup;
mod transform;
mod transport;

use protocol::{Direction, Frame};
use schema::AcpV1Contract;
use transform::{TransformOptions, TransformPipeline};
use transport::{FrameObserver, RelayFinishReason, StdioRelay};

pub use setup::{apply_acp_setup_plan, build_acp_setup_plan, AcpSetupReport};

const ACP_CAPTURE_LOCK_WAIT: Duration = Duration::from_secs(30);
const CLAUDE_ACP_ADAPTER: &str = "@agentclientprotocol/claude-agent-acp@latest";
const CODEX_ACP_ADAPTER: &str = "@agentclientprotocol/codex-acp@latest";
const ACP_MAX_PENDING_EVENTS_PER_TURN: usize = 128;
const ACP_MAX_ASSISTANT_MESSAGE_BYTES: usize = 256 * 1024;
const ACP_MAX_ASSISTANT_TOTAL_BYTES: usize = 1024 * 1024;

#[derive(Clone, Debug)]
pub struct AcpRelayOptions {
    pub workspace_root: PathBuf,
    pub db_dir: PathBuf,
    pub lane: Option<String>,
    pub from_ref: Option<String>,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub materialize: bool,
    pub workdir: Option<PathBuf>,
    pub inject_mcp: bool,
    pub upstream_command: Vec<String>,
    pub upstream_env: BTreeMap<String, String>,
}

/// A resolved ACP provider profile and the command Trail will launch upstream.
#[derive(Clone, Debug)]
pub struct AcpProviderLaunch {
    pub profile: AcpProviderProfile,
    pub upstream_command: Vec<String>,
    pub upstream_env: BTreeMap<String, String>,
}

pub fn acp_provider_profile(agent: &str) -> Result<AcpProviderProfile> {
    match canonical_acp_agent(agent) {
        Some("claude-code") => Ok(npx_acp_profile(
            "claude-code",
            "Claude Code",
            "uses the Claude ACP adapter through npx",
            Some(vec!["claude".to_string()]),
        )),
        Some("codex") => Ok(npx_acp_profile(
            "codex",
            "Codex",
            "uses the Codex ACP adapter through npx",
            Some(vec!["codex".to_string()]),
        )),
        Some("cursor") => {
            let available = command_in_path("agent");
            Ok(AcpProviderProfile {
                agent: "cursor".to_string(),
                display_name: "Cursor".to_string(),
                available,
                relay_command: built_in_acp_relay_command("cursor"),
                notes: if available {
                    vec!["uses the Cursor CLI ACP server through `agent acp`".to_string()]
                } else {
                    vec!["`agent` was not found on PATH".to_string()]
                },
                supports_acp: true,
                supports_mcp: true,
                supports_terminal: true,
                default_terminal_command: Some(vec!["agent".to_string()]),
            })
        }
        Some("grok") => {
            let available = command_in_path("grok");
            Ok(AcpProviderProfile {
                agent: "grok".to_string(),
                display_name: "Grok Build".to_string(),
                available,
                relay_command: built_in_acp_relay_command("grok"),
                notes: if available {
                    vec!["uses Grok Build's native ACP server through `grok agent stdio`"
                        .to_string()]
                } else {
                    vec!["`grok` was not found on PATH".to_string()]
                },
                supports_acp: true,
                supports_mcp: true,
                supports_terminal: true,
                default_terminal_command: Some(vec!["grok".to_string()]),
            })
        }
        _ => Err(Error::InvalidInput(format!(
            "unsupported ACP agent `{agent}`; supported agents: {}; use `trail acp relay -- <COMMAND>...` for another ACP-compatible agent",
            supported_acp_agents().join(", ")
        ))),
    }
}

/// Returns the upstream ACP command for one of Trail's built-in ACP providers.
///
/// This keeps the user-facing relay command short (`trail acp relay codex`) while
/// retaining an explicit command path for custom ACP-compatible agents.
pub fn acp_provider_upstream_command(agent: &str) -> Result<Vec<String>> {
    let profile = acp_provider_profile(agent)?;
    match profile.agent.as_str() {
        "claude-code" => Ok(vec![
            "npx".to_string(),
            "-y".to_string(),
            CLAUDE_ACP_ADAPTER.to_string(),
        ]),
        "codex" => Ok(vec![
            "npx".to_string(),
            "-y".to_string(),
            CODEX_ACP_ADAPTER.to_string(),
        ]),
        "cursor" => Ok(vec!["agent".to_string(), "acp".to_string()]),
        "grok" => Ok(vec![
            "grok".to_string(),
            "agent".to_string(),
            "stdio".to_string(),
        ]),
        _ => Err(Error::InvalidInput(format!(
            "provider `{}` does not define an ACP upstream command",
            profile.agent
        ))),
    }
}

/// Resolves either a built-in provider alias or an agent from the official ACP registry.
///
/// Registry agents are fetched from the official index and cached below `cache_dir`
/// when supplied. Registry `npx` and `uvx` agents use their package runner directly;
/// matching binary distributions are downloaded into the cache on first launch.
pub fn resolve_acp_provider(
    agent: &str,
    cache_dir: Option<&std::path::Path>,
) -> Result<AcpProviderLaunch> {
    if let Ok(profile) = acp_provider_profile(agent) {
        return Ok(AcpProviderLaunch {
            upstream_command: acp_provider_upstream_command(&profile.agent)?,
            profile,
            upstream_env: BTreeMap::new(),
        });
    }
    registry::resolve_registry_provider(agent, cache_dir)
}

/// Resolves a built-in or registry provider profile without downloading a binary.
pub fn acp_provider_profile_with_registry(
    agent: &str,
    cache_dir: Option<&std::path::Path>,
) -> Result<AcpProviderProfile> {
    acp_provider_profile(agent).or_else(|_| registry::registry_provider_profile(agent, cache_dir))
}

pub fn acp_provider_profiles() -> Vec<AcpProviderProfile> {
    supported_acp_agents()
        .into_iter()
        .filter_map(|agent| acp_provider_profile(agent).ok())
        .collect()
}

/// Lists Trail's built-in profiles plus every agent currently in the official ACP registry.
pub fn acp_provider_profiles_with_registry(
    cache_dir: Option<&std::path::Path>,
) -> Result<Vec<AcpProviderProfile>> {
    // Listing providers should keep working offline; named registry agents still
    // return a clear resolution error until a live or cached registry is available.
    Ok(registry::registry_provider_profiles(cache_dir).unwrap_or_else(|_| acp_provider_profiles()))
}

pub fn agent_provider_profile(provider: &str) -> Result<AcpProviderProfile> {
    if let Ok(profile) = acp_provider_profile(provider) {
        return Ok(profile);
    }
    match canonical_agent_provider(provider) {
        Some("gemini") => Ok(terminal_provider_profile(
            "gemini",
            "Gemini CLI",
            vec!["gemini".to_string()],
            true,
            "runs Gemini CLI in a Trail materialized task lane",
        )),
        Some("aider") => Ok(terminal_provider_profile(
            "aider",
            "Aider",
            vec!["aider".to_string()],
            false,
            "runs Aider in a Trail materialized task lane",
        )),
        Some("opencode") => Ok(terminal_provider_profile(
            "opencode",
            "OpenCode",
            vec!["opencode".to_string()],
            false,
            "runs OpenCode in a Trail materialized task lane",
        )),
        _ => Err(Error::InvalidInput(format!(
            "unsupported agent provider `{provider}`; supported providers: {}. You can still pass an explicit command after `--` to `trail agent start` or `trail agent acp run`.",
            supported_agent_providers().join(", ")
        ))),
    }
}

pub fn agent_provider_profiles() -> Vec<AcpProviderProfile> {
    supported_agent_providers()
        .into_iter()
        .filter_map(|agent| agent_provider_profile(agent).ok())
        .collect()
}

pub fn terminal_agent_command(provider: &str) -> Result<Vec<String>> {
    agent_provider_profile(provider)?
        .default_terminal_command
        .ok_or_else(|| {
            Error::InvalidInput(format!(
                "provider `{provider}` does not define a default terminal command; pass one after `--`"
            ))
        })
}

fn supported_acp_agents() -> Vec<&'static str> {
    vec!["claude-code", "codex", "cursor", "grok"]
}

fn supported_agent_providers() -> Vec<&'static str> {
    vec![
        "claude-code",
        "codex",
        "cursor",
        "gemini",
        "aider",
        "opencode",
    ]
}

fn canonical_acp_agent(agent: &str) -> Option<&'static str> {
    match agent {
        "claude-code" | "claude" => Some("claude-code"),
        "codex" | "codex-cli" | "openai-codex" => Some("codex"),
        "cursor" | "cursor-agent" => Some("cursor"),
        "grok" | "grok-build" | "xai-grok" => Some("grok"),
        _ => None,
    }
}

fn canonical_agent_provider(provider: &str) -> Option<&'static str> {
    match provider {
        "gemini" | "gemini-cli" => Some("gemini"),
        "aider" => Some("aider"),
        "opencode" | "open-code" => Some("opencode"),
        other => canonical_acp_agent(other),
    }
}

fn npx_acp_profile(
    provider: &str,
    display_name: &str,
    available_note: &str,
    terminal_command: Option<Vec<String>>,
) -> AcpProviderProfile {
    let available = command_in_path("npx");
    AcpProviderProfile {
        agent: provider.to_string(),
        display_name: display_name.to_string(),
        available,
        relay_command: built_in_acp_relay_command(provider),
        notes: if available {
            vec![available_note.to_string()]
        } else {
            vec!["`npx` was not found on PATH".to_string()]
        },
        supports_acp: true,
        supports_mcp: true,
        supports_terminal: terminal_command.is_some(),
        default_terminal_command: terminal_command,
    }
}

fn terminal_provider_profile(
    provider: &str,
    display_name: &str,
    terminal_command: Vec<String>,
    supports_mcp: bool,
    available_note: &str,
) -> AcpProviderProfile {
    let launcher = terminal_command
        .first()
        .map(String::as_str)
        .unwrap_or(provider);
    let available = command_in_path(launcher);
    AcpProviderProfile {
        agent: provider.to_string(),
        display_name: display_name.to_string(),
        available,
        relay_command: Vec::new(),
        notes: if available {
            vec![available_note.to_string()]
        } else {
            vec![format!("`{launcher}` was not found on PATH")]
        },
        supports_acp: false,
        supports_mcp,
        supports_terminal: true,
        default_terminal_command: Some(terminal_command),
    }
}

pub(crate) fn built_in_acp_relay_command(provider: &str) -> Vec<String> {
    vec![
        "trail".to_string(),
        "acp".to_string(),
        "relay".to_string(),
        provider.to_string(),
    ]
}

pub fn run_stdio_relay(options: AcpRelayOptions) -> Result<()> {
    let coordinator = Arc::new(Mutex::new(CaptureCoordinator::new(options.clone())?));
    let pipeline = TransformPipeline::new(
        Arc::new(AcpV1Contract::load()?),
        TransformOptions::from_relay(&options),
    );
    let observer = Arc::new(CaptureObserver {
        coordinator,
        pipeline: Mutex::new(pipeline),
    });
    StdioRelay::new(observer).run(&options)
}

struct CaptureObserver {
    coordinator: Arc<Mutex<CaptureCoordinator>>,
    pipeline: Mutex<TransformPipeline>,
}

impl FrameObserver for CaptureObserver {
    fn observe(&self, frame: &mut Frame) -> Result<()> {
        let outcome = self
            .pipeline
            .lock()
            .map_err(|_| Error::InvalidInput("ACP transform lock poisoned".to_string()))?
            .apply(frame)?;
        if let Some(diagnostic) = outcome.diagnostic_message() {
            eprintln!("trail acp relay negotiation warning: {diagnostic}");
        }
        if !outcome.capture_v1() {
            return Ok(());
        }

        let mut candidate = frame.value().clone();
        let captured = capture_step(&self.coordinator, |capture| match frame.direction() {
            Direction::ClientToAgent => capture.before_client_message(&mut candidate),
            Direction::AgentToClient => capture.before_agent_message(&mut candidate),
        });
        if captured && candidate != *frame.value() {
            if let Err(error) = self
                .pipeline
                .lock()
                .map_err(|_| Error::InvalidInput("ACP transform lock poisoned".to_string()))?
                .commit_candidate(frame, candidate)
            {
                eprintln!("trail acp relay transformation warning: {error}");
            }
        }
        Ok(())
    }

    fn finish(&self, reason: RelayFinishReason) {
        let (status, summary) = match reason {
            RelayFinishReason::EditorEof => ("cancelled", "editor input closed".to_string()),
            RelayFinishReason::EditorError(error) => {
                ("failed", format!("editor sent malformed ACP: {error}"))
            }
            RelayFinishReason::AgentEof => ("failed", "upstream output closed".to_string()),
            RelayFinishReason::AgentError(error) => {
                ("failed", format!("upstream sent malformed JSON: {error}"))
            }
        };
        let _ = capture_step(&self.coordinator, |capture| {
            capture.finish_open_turns(status, &summary)
        });
    }
}

pub(crate) fn command_in_path(command: &str) -> bool {
    if command.contains(std::path::MAIN_SEPARATOR) {
        return PathBuf::from(command).is_file();
    }
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|dir| dir.join(command).is_file())
}

fn record_acp_lifecycle_event(
    db: &mut Trail,
    lane: &str,
    session_id: &str,
    turn_id: Option<&str>,
    provider: Option<&str>,
    acp_session_id: &str,
    kind: AgentLifecycleEventKind,
    payload: Value,
) -> Result<()> {
    let provider = provider.unwrap_or("acp-agent").to_ascii_lowercase();
    let payload = redact_json(payload);
    let payload_bytes = serde_json::to_vec(&payload)?;
    let event_id = format!(
        "acp_event_{}",
        crate::ids::short_hash(
            format!(
                "{}:{}:{}:{}:{}",
                db.config().workspace.id.0,
                session_id,
                turn_id.unwrap_or("session"),
                kind.wire_name(),
                hex::encode(Sha256::digest(&payload_bytes))
            )
            .as_bytes(),
            24,
        )
    );
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    let event = AgentLifecycleEvent {
        schema: AGENT_LIFECYCLE_EVENT_SCHEMA.to_string(),
        version: AGENT_LIFECYCLE_EVENT_VERSION,
        event_id: event_id.clone(),
        event_type: AgentLifecycleEventType::from(kind),
        occurred_at: Some(now),
        received_at: now,
        provider,
        provider_version: None,
        transport: AgentCaptureTransport::Acp,
        workspace_id: db.config().workspace.id.0.clone(),
        lane_id: Some(db.lane_branch(lane)?.lane_id),
        capture_run_id: None,
        native: AgentNativeEventIdentity {
            session_id: Some(acp_session_id.to_string()),
            turn_id: None,
            message_id: payload
                .get("message_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            tool_id: payload
                .get("tool_id")
                .and_then(Value::as_str)
                .map(str::to_string),
            subagent_id: None,
            event_name: format!("acp/{}", kind.wire_name()),
            sequence: None,
        },
        correlation: AgentEventCorrelation {
            trace_id: turn_id.map(acp_trace_id_for_turn),
            ..AgentEventCorrelation::default()
        },
        payload,
        evidence: AgentEventEvidence {
            receipt_id: format!("acp:{event_id}"),
            raw_digest: Some(format!(
                "sha256:{}",
                hex::encode(Sha256::digest(payload_bytes))
            )),
            transcript_offset: None,
            confidence: AgentEvidenceConfidence::ProtocolStructured,
        },
    };
    event.validate()?;
    let serialized = serde_json::to_value(&event)?;
    if let Some(turn_id) = turn_id {
        db.add_lane_turn_event(turn_id, kind.wire_name(), Some(serialized), None, None)?;
    } else {
        db.add_lane_session_event(lane, session_id, kind.wire_name(), Some(serialized))?;
    }
    Ok(())
}

fn capture_step<F>(coordinator: &Arc<Mutex<CaptureCoordinator>>, f: F) -> bool
where
    F: FnOnce(&mut CaptureCoordinator) -> Result<()>,
{
    match coordinator.lock() {
        Ok(mut capture) => {
            let result = Trail::with_write_lock_wait(ACP_CAPTURE_LOCK_WAIT, || f(&mut capture));
            if let Err(err) = result {
                eprintln!("trail acp relay capture warning: {err}");
                return false;
            }
            true
        }
        Err(_) => {
            eprintln!("trail acp relay capture warning: capture coordinator lock poisoned");
            false
        }
    }
}

#[derive(Clone, Debug)]
struct SessionState {
    acp_session_id: String,
    upstream_session_id: Option<String>,
    lane_name: String,
    trail_session_id: String,
    original_cwd: String,
    effective_cwd: String,
    materialized: bool,
}

#[derive(Clone, Debug)]
struct PendingSession {
    method: String,
    session: SessionState,
}

#[derive(Clone, Debug)]
struct PendingPrompt {
    acp_session_id: String,
    lane_name: String,
    turn_id: String,
    root_span_id: Option<String>,
    materialized: bool,
}

#[derive(Clone, Debug)]
struct ActiveTurn {
    lane_name: String,
    trail_session_id: String,
    turn_id: String,
    root_span_id: Option<String>,
    effective_cwd: String,
    materialized: bool,
    assistant_buffers: HashMap<String, String>,
    assistant_buffer_order: Vec<String>,
    assistant_message_bytes: HashMap<String, usize>,
    tool_spans: HashMap<String, String>,
    structured_diff_keys: HashSet<String>,
    pending_events: Vec<BufferedTurnEvent>,
    usage: TurnEnvelopeUsage,
    event_count: u64,
    tool_event_count: u64,
    structured_diff_count: u64,
    redaction_applied: bool,
    assistant_buffer_bytes: usize,
    capture_truncated: bool,
}

#[derive(Clone, Debug)]
struct BufferedTurnEvent {
    event_type: String,
    payload: Option<Value>,
    change_id: Option<String>,
    message_id: Option<String>,
}

impl ActiveTurn {
    fn append_assistant_text(&mut self, message_id: String, text: &str) {
        if !self.assistant_buffers.contains_key(&message_id) {
            self.assistant_buffer_order.push(message_id.clone());
        }
        self.assistant_buffers
            .entry(message_id.clone())
            .or_default()
            .push_str(text);
        *self.assistant_message_bytes.entry(message_id).or_default() += text.len();
        self.assistant_buffer_bytes += text.len();
    }

    fn assistant_message_bytes(&self, message_id: &str) -> usize {
        self.assistant_message_bytes
            .get(message_id)
            .copied()
            .unwrap_or_default()
    }

    fn drain_assistant_buffers(&mut self) -> Vec<(String, String)> {
        let ordered = std::mem::take(&mut self.assistant_buffer_order);
        let mut drained = Vec::new();
        for message_id in ordered {
            if let Some(text) = self.assistant_buffers.remove(&message_id) {
                drained.push((message_id, text));
            }
        }
        for (message_id, text) in std::mem::take(&mut self.assistant_buffers) {
            drained.push((message_id, text));
        }
        drained
    }

    fn push_event(&mut self, event_type: impl Into<String>, payload: Option<Value>) {
        let event_type = event_type.into();
        if payload.as_ref().is_some_and(value_has_redaction_marker) {
            self.redaction_applied = true;
        }
        self.event_count += 1;
        if matches!(
            event_type.as_str(),
            "tool_call" | "tool_call_update" | "span_started" | "span_ended"
        ) {
            self.tool_event_count += 1;
        }
        self.pending_events.push(BufferedTurnEvent {
            event_type,
            payload,
            change_id: None,
            message_id: None,
        });
    }

    fn push_truncation_event(&mut self, reason: &str) {
        if self.capture_truncated {
            return;
        }
        self.capture_truncated = true;
        self.push_event(
            "acp_capture_truncated",
            Some(redact_json(serde_json::json!({
                "protocol": "acp",
                "reason": reason
            }))),
        );
    }

    fn push_span_started(&mut self, span_id: &str, name: &str, attributes: Option<Value>) {
        self.push_event(
            "span_started",
            Some(redact_json(serde_json::json!({
                "span_id": span_id,
                "trace_id": acp_trace_id_for_turn(&self.turn_id),
                "parent_span_id": self.root_span_id,
                "span_type": "tool",
                "name": name,
                "attributes": attributes.unwrap_or(Value::Null)
            }))),
        );
    }

    fn push_span_ended(&mut self, span_id: &str, status: &str, result: Option<Value>) {
        self.push_event(
            "span_ended",
            Some(redact_json(serde_json::json!({
                "span_id": span_id,
                "trace_id": acp_trace_id_for_turn(&self.turn_id),
                "status": status,
                "result": result.unwrap_or(Value::Null)
            }))),
        );
    }
}

#[derive(Clone, Debug)]
struct PendingPermission {
    approval_id: Option<String>,
    options_by_id: HashMap<String, String>,
}

struct CaptureCoordinator {
    options: AcpRelayOptions,
    pending_sessions: HashMap<String, PendingSession>,
    pending_prompts: HashMap<String, PendingPrompt>,
    pending_closes: HashMap<String, String>,
    pending_permissions: HashMap<String, PendingPermission>,
    sessions_by_acp: HashMap<String, SessionState>,
    active_turns: HashMap<String, ActiveTurn>,
    upstream_command_json: Option<String>,
}

impl CaptureCoordinator {
    fn new(options: AcpRelayOptions) -> Result<Self> {
        let upstream_command_json =
            serde_json::to_string(&redact_command(&options.upstream_command)).ok();
        Ok(Self {
            options,
            pending_sessions: HashMap::new(),
            pending_prompts: HashMap::new(),
            pending_closes: HashMap::new(),
            pending_permissions: HashMap::new(),
            sessions_by_acp: HashMap::new(),
            active_turns: HashMap::new(),
            upstream_command_json,
        })
    }

    fn before_client_message(&mut self, message: &mut Value) -> Result<()> {
        if method_name(message).is_none() {
            self.capture_client_response(message)?;
            return Ok(());
        }

        match method_name(message) {
            Some("initialize") => {}
            Some("session/new") | Some("session/load") | Some("session/resume") => {
                self.prepare_session_request(message)?;
            }
            Some("session/prompt") => {
                self.prepare_prompt_request(message)?;
            }
            Some("session/cancel") => {
                self.capture_cancel(message)?;
            }
            Some("session/close") => {
                self.prepare_close_request(message)?;
            }
            _ => {}
        }
        Ok(())
    }

    fn before_agent_message(&mut self, message: &mut Value) -> Result<()> {
        if method_name(message).is_none() {
            self.capture_agent_response(message)?;
            return Ok(());
        }

        match method_name(message) {
            Some("session/update") => self.capture_session_update(message)?,
            Some("session/request_permission") => self.capture_permission_request(message)?,
            _ => {}
        }
        Ok(())
    }

    fn capture_client_response(&mut self, message: &Value) -> Result<()> {
        let Some(id) = rpc_id_key(message) else {
            return Ok(());
        };
        let Some(permission) = self.pending_permissions.remove(&id) else {
            return Ok(());
        };
        let Some(approval_id) = permission.approval_id else {
            return Ok(());
        };
        let decision = permission_decision(message, &permission.options_by_id);
        let mut db = self.open_db()?;
        db.decide_lane_approval(
            &approval_id,
            decision,
            Some("acp-editor".to_string()),
            Some("mirrored from ACP permission response".to_string()),
        )?;
        Ok(())
    }

    fn capture_agent_response(&mut self, message: &mut Value) -> Result<()> {
        let Some(id) = rpc_id_key(message) else {
            return Ok(());
        };

        if let Some(pending) = self.pending_sessions.remove(&id) {
            self.finish_session_request(message, pending)?;
            return Ok(());
        }

        if let Some(pending) = self.pending_prompts.remove(&id) {
            self.finish_prompt_request(message, pending)?;
            return Ok(());
        }

        if let Some(acp_session_id) = self.pending_closes.remove(&id) {
            self.finish_close_request(message, &acp_session_id)?;
        }

        Ok(())
    }

    fn prepare_session_request(&mut self, message: &mut Value) -> Result<()> {
        let method = method_name(message).unwrap_or_default().to_string();
        let request_id = rpc_id_key(message);
        let params = params_object_mut(message)?;
        let original_cwd = params
            .get("cwd")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| self.options.workspace_root.to_string_lossy().to_string());
        let requested_acp_session_id = params
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::to_string);
        let session = self.ensure_capture_session(
            &method,
            requested_acp_session_id.as_deref(),
            &original_cwd,
        )?;

        if session.effective_cwd != original_cwd {
            params.insert(
                "cwd".to_string(),
                Value::String(session.effective_cwd.clone()),
            );
        }
        if self.options.inject_mcp {
            inject_trail_mcp_server(params, &self.options)?;
        }

        if let Some(request_id) = request_id {
            self.pending_sessions.insert(
                request_id,
                PendingSession {
                    method,
                    session: session.clone(),
                },
            );
        }
        if let Some(acp_session_id) = requested_acp_session_id {
            self.sessions_by_acp.insert(acp_session_id, session);
        }
        Ok(())
    }

    fn ensure_capture_session(
        &mut self,
        method: &str,
        acp_session_id: Option<&str>,
        original_cwd: &str,
    ) -> Result<SessionState> {
        if let Some(acp_session_id) = acp_session_id {
            if let Some(existing) = self.sessions_by_acp.get(acp_session_id) {
                return Ok(existing.clone());
            }
            let db = self.open_db()?;
            if let Some(mapping) = db.try_lane_acp_session(acp_session_id)? {
                let lane_name = db.resolve_lane_handle(&mapping.lane_id)?;
                let effective_cwd =
                    self.materialized_cwd_for_existing_lane(&lane_name, original_cwd)?;
                let state = SessionState {
                    acp_session_id: acp_session_id.to_string(),
                    upstream_session_id: mapping.upstream_session_id,
                    lane_name,
                    trail_session_id: mapping.trail_session_id,
                    original_cwd: original_cwd.to_string(),
                    materialized: effective_cwd != original_cwd,
                    effective_cwd,
                };
                return Ok(state);
            }
        }

        let lane_name = self.resolve_lane_name(acp_session_id, original_cwd);
        let mut db = self.open_db()?;
        match db.lane_details(&lane_name) {
            Ok(_) => {
                if self.options.materialize {
                    db.ensure_lane_workdir_materialized(&lane_name, self.options.workdir.clone())?;
                }
            }
            Err(Error::RefNotFound(_)) => {
                db.spawn_lane_with_workdir(
                    &lane_name,
                    self.options.from_ref.as_deref(),
                    self.options.materialize,
                    self.options.provider.clone(),
                    self.options.model.clone(),
                    self.options.workdir.clone(),
                )?;
            }
            Err(err) => return Err(err),
        }

        let details = db.lane_details(&lane_name)?;
        let effective_cwd = if self.options.materialize {
            details
                .branch
                .workdir
                .clone()
                .unwrap_or_else(|| original_cwd.to_string())
        } else {
            original_cwd.to_string()
        };
        let title = Some(format!("ACP {method}"));
        let session = db.start_lane_session(&lane_name, title, None)?.session;
        db.add_lane_session_event(
            &lane_name,
            &session.session_id,
            "acp_session_starting",
            Some(redact_json(serde_json::json!({
                "protocol": "acp",
                "method": method,
                "requested_acp_session_id": acp_session_id,
                "cwd": original_cwd,
                "effective_cwd": effective_cwd,
                "provider": self.options.provider,
                "model": self.options.model,
                "materialized": self.options.materialize
            }))),
        )?;
        Ok(SessionState {
            acp_session_id: acp_session_id.map(str::to_string).unwrap_or_else(|| {
                format!(
                    "pending-{}",
                    crate::ids::short_hash(session.session_id.as_bytes(), 8)
                )
            }),
            upstream_session_id: None,
            lane_name,
            trail_session_id: session.session_id,
            original_cwd: original_cwd.to_string(),
            effective_cwd,
            materialized: self.options.materialize && details.branch.workdir.is_some(),
        })
    }

    fn materialized_cwd_for_existing_lane(
        &self,
        lane_name: &str,
        original_cwd: &str,
    ) -> Result<String> {
        if !self.options.materialize {
            return Ok(original_cwd.to_string());
        }
        let mut db = self.open_db()?;
        let report =
            db.ensure_lane_workdir_materialized(lane_name, self.options.workdir.clone())?;
        Ok(report.workdir.unwrap_or_else(|| original_cwd.to_string()))
    }

    fn finish_session_request(&mut self, message: &Value, pending: PendingSession) -> Result<()> {
        let status = if message.get("error").is_some() {
            "failed"
        } else {
            match pending.method.as_str() {
                "session/load" => "loaded",
                "session/resume" => "resumed",
                _ => "active",
            }
        };
        let acp_session_id = response_session_id(message)
            .or_else(|| {
                (pending.session.acp_session_id != "pending")
                    .then_some(pending.session.acp_session_id.as_str())
            })
            .unwrap_or(&pending.session.acp_session_id)
            .to_string();
        let mut session = pending.session.clone();
        session.acp_session_id = acp_session_id.clone();
        session.upstream_session_id = response_session_id(message).map(str::to_string);

        let mut db = self.open_db()?;
        db.upsert_lane_acp_session(
            &acp_session_id,
            response_session_id(message),
            &session.lane_name,
            &session.trail_session_id,
            &session.effective_cwd,
            self.options.provider.as_deref(),
            self.options.model.as_deref(),
            self.upstream_command_json.as_deref(),
            status,
        )?;
        db.add_lane_session_event(
            &session.lane_name,
            &session.trail_session_id,
            event_for_session_status(status),
            Some(redact_json(serde_json::json!({
                "protocol": "acp",
                "method": pending.method,
                "acp_session_id": acp_session_id,
                "upstream_session_id": response_session_id(message),
                "cwd": session.original_cwd,
                "effective_cwd": session.effective_cwd,
                "status": status
            }))),
        )?;
        record_acp_lifecycle_event(
            &mut db,
            &session.lane_name,
            &session.trail_session_id,
            None,
            self.options.provider.as_deref(),
            &acp_session_id,
            if status == "resumed" || status == "loaded" {
                AgentLifecycleEventKind::SessionResumed
            } else if status == "failed" {
                AgentLifecycleEventKind::Diagnostic
            } else {
                AgentLifecycleEventKind::SessionStarted
            },
            serde_json::json!({"method": pending.method, "status": status}),
        )?;
        if status == "failed" {
            let _ = db.end_lane_session(&session.trail_session_id, "failed");
        }
        self.sessions_by_acp.insert(acp_session_id, session);
        Ok(())
    }

    fn prepare_prompt_request(&mut self, message: &Value) -> Result<()> {
        let Some(request_id) = rpc_id_key(message) else {
            return Ok(());
        };
        let params = params_object(message)?;
        let acp_session_id = params
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                Error::InvalidInput("ACP session/prompt missing sessionId".to_string())
            })?;
        let session = self.resolve_session_state(acp_session_id)?;
        let prompt_text = prompt_text(params.get("prompt"));
        let mut db = self.open_db()?;
        let branch = db.lane_branch(&session.lane_name)?;
        let initial_envelope = TurnEnvelope::new_acp_prompt(TurnEnvelopeAcpPromptInput {
            provider: self.options.provider.clone(),
            model: self.options.model.clone(),
            trail_session_id: session.trail_session_id.clone(),
            acp_session_id: acp_session_id.to_string(),
            upstream_session_id: session.upstream_session_id.clone(),
            upstream_command_hash: self.upstream_command_hash(),
            prompt_hash: prompt_hash(&prompt_text),
            prompt_summary: summarize_text(&prompt_text),
            user_message_id: None,
            lane: session.lane_name.clone(),
            cwd: session.original_cwd.clone(),
            effective_cwd: session.effective_cwd.clone(),
            workdir_mode: if session.materialized {
                "materialized".to_string()
            } else {
                "virtual".to_string()
            },
            base_change: branch.base_change.clone(),
            before_change: branch.head_change.clone(),
        });
        let turn = db.begin_lane_session_turn(
            &session.lane_name,
            &session.trail_session_id,
            Some(initial_envelope.to_metadata_value()),
        )?;
        let user_message = db.add_lane_turn_message(&turn.turn.turn_id, "user", &prompt_text)?;
        let mut envelope = initial_envelope;
        envelope.prompt.user_message_id = Some(user_message.message_id.clone());
        db.update_lane_turn_metadata(&turn.turn.turn_id, &envelope.to_metadata_value())?;
        db.add_lane_turn_event(
            &turn.turn.turn_id,
            "acp_prompt_started",
            Some(redact_json(serde_json::json!({
                "protocol": "acp",
                "acp_session_id": acp_session_id,
                "request_id": message.get("id").cloned(),
                "prompt_summary": summarize_text(&prompt_text),
            }))),
            None,
            None,
        )?;
        record_acp_lifecycle_event(
            &mut db,
            &session.lane_name,
            &session.trail_session_id,
            Some(&turn.turn.turn_id),
            self.options.provider.as_deref(),
            acp_session_id,
            AgentLifecycleEventKind::TurnStarted,
            serde_json::json!({"prompt_summary": summarize_text(&prompt_text)}),
        )?;
        record_acp_lifecycle_event(
            &mut db,
            &session.lane_name,
            &session.trail_session_id,
            Some(&turn.turn.turn_id),
            self.options.provider.as_deref(),
            acp_session_id,
            AgentLifecycleEventKind::MessageUser,
            serde_json::json!({
                "message_id": user_message.message_id,
                "prompt_hash": prompt_hash(&prompt_text),
            }),
        )?;
        let root_span = db
            .start_lane_trace_span(
                &turn.turn.turn_id,
                "prompt",
                "ACP prompt turn",
                None,
                None,
                Some(redact_json(serde_json::json!({
                    "protocol": "acp",
                    "acp_session_id": acp_session_id,
                    "provider": self.options.provider,
                    "model": self.options.model
                }))),
            )
            .ok()
            .map(|report| report.span.span_id);

        let pending = PendingPrompt {
            acp_session_id: acp_session_id.to_string(),
            lane_name: session.lane_name.clone(),
            turn_id: turn.turn.turn_id.clone(),
            root_span_id: root_span.clone(),
            materialized: session.materialized,
        };
        self.pending_prompts.insert(request_id, pending.clone());
        self.active_turns.insert(
            acp_session_id.to_string(),
            ActiveTurn {
                lane_name: pending.lane_name,
                trail_session_id: session.trail_session_id,
                turn_id: pending.turn_id,
                root_span_id: pending.root_span_id,
                effective_cwd: session.effective_cwd,
                materialized: pending.materialized,
                assistant_buffers: HashMap::new(),
                assistant_buffer_order: Vec::new(),
                assistant_message_bytes: HashMap::new(),
                tool_spans: HashMap::new(),
                structured_diff_keys: HashSet::new(),
                pending_events: Vec::new(),
                usage: TurnEnvelopeUsage::default(),
                event_count: 0,
                tool_event_count: 0,
                structured_diff_count: 0,
                redaction_applied: false,
                assistant_buffer_bytes: 0,
                capture_truncated: false,
            },
        );
        Ok(())
    }

    fn finish_prompt_request(&mut self, message: &Value, pending: PendingPrompt) -> Result<()> {
        let status = prompt_status(message);
        let mut active = self.active_turns.remove(&pending.acp_session_id);
        let mut db = self.open_db()?;
        if let Some(active_turn) = active.as_mut() {
            let open_spans = active_turn.tool_spans.drain().collect::<Vec<_>>();
            for (_, span_id) in open_spans {
                active_turn.push_span_ended(&span_id, status, None);
            }
            self.flush_turn_events(active_turn)?;
            self.flush_assistant_messages(active_turn, "prompt_completed")?;
        }
        if pending.materialized {
            let _ = db.record_lane_workdir_for_turn(
                &pending.lane_name,
                &pending.turn_id,
                Some("ACP prompt workdir checkpoint".to_string()),
            );
        }
        db.add_lane_turn_event(
            &pending.turn_id,
            "acp_prompt_finished",
            Some(redact_json(serde_json::json!({
                "protocol": "acp",
                "acp_session_id": pending.acp_session_id,
                "status": status,
                "stop_reason": stop_reason(message),
                "error": message.get("error").cloned()
            }))),
            None,
            None,
        )?;
        let terminal_kind = match status {
            "failed" => AgentLifecycleEventKind::TurnFailed,
            "cancelled" | "interrupted" => AgentLifecycleEventKind::TurnCancelled,
            _ => AgentLifecycleEventKind::TurnCompleted,
        };
        let session_id = db
            .lane_turn(&pending.turn_id)?
            .session_id
            .ok_or_else(|| Error::Corrupt("ACP turn lost its session identity".to_string()))?;
        record_acp_lifecycle_event(
            &mut db,
            &pending.lane_name,
            &session_id,
            Some(&pending.turn_id),
            self.options.provider.as_deref(),
            &pending.acp_session_id,
            terminal_kind,
            serde_json::json!({"status": status, "stop_reason": stop_reason(message)}),
        )?;
        if let Some(span_id) = pending.root_span_id {
            let _ = db.end_lane_trace_span(
                &span_id,
                status,
                Some(redact_json(serde_json::json!({
                    "stop_reason": stop_reason(message)
                }))),
            );
        }
        db.end_lane_turn(&pending.turn_id, status)?;
        db.create_turn_evidence_manifest(&pending.turn_id)?;
        db.classify_session_activity(&session_id, 10_000)?;
        self.finalize_turn_envelope(
            &mut db,
            &pending.turn_id,
            status,
            stop_reason(message).map(str::to_string),
            error_summary(message),
            active.as_ref(),
        )?;
        Ok(())
    }

    fn capture_session_update(&mut self, message: &Value) -> Result<()> {
        let params = params_object(message)?;
        let Some(acp_session_id) = params.get("sessionId").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(update) = params.get("update").and_then(Value::as_object) else {
            return Ok(());
        };
        let update_kind = update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .unwrap_or("unknown");
        let Some(mut active) = self.active_turns.remove(acp_session_id) else {
            self.capture_session_update_without_turn(acp_session_id, update_kind, update)?;
            return Ok(());
        };

        let result = match update_kind {
            "agent_message_chunk" => {
                let key = update
                    .get("messageId")
                    .and_then(Value::as_str)
                    .unwrap_or("default")
                    .to_string();
                let text = content_text(update.get("content"));
                if !text.is_empty() {
                    let redaction_truncated = text.contains("\n[truncated]");
                    let current_message_len = active.assistant_message_bytes(&key);
                    let available_for_message =
                        ACP_MAX_ASSISTANT_MESSAGE_BYTES.saturating_sub(current_message_len);
                    let available_for_turn =
                        ACP_MAX_ASSISTANT_TOTAL_BYTES.saturating_sub(active.assistant_buffer_bytes);
                    let allowed =
                        utf8_prefix_len(&text, available_for_message.min(available_for_turn));
                    if allowed > 0 {
                        active.append_assistant_text(key, &text[..allowed]);
                    }
                    if redaction_truncated || allowed < text.len() {
                        active.push_truncation_event("assistant message buffer limit exceeded");
                    }
                }
                Ok(())
            }
            "tool_call" => self.capture_tool_call_start(&mut active, update),
            "tool_call_update" => self.capture_tool_call_update(&mut active, update),
            "plan" => {
                self.flush_turn_events(&mut active)?;
                self.flush_assistant_messages(&mut active, "before_plan_update")?;
                active.push_event(
                    "plan_update",
                    Some(redact_json(Value::Object(update.clone()))),
                );
                Ok(())
            }
            kind if ignore_session_update(kind) => Ok(()),
            "available_commands_update" => {
                self.flush_turn_events(&mut active)?;
                self.flush_assistant_messages(&mut active, "before_available_commands_update")?;
                active.push_event(
                    "acp_available_commands_update",
                    Some(summarize_available_commands(update)),
                );
                Ok(())
            }
            "usage_update" => {
                active.usage = turn_envelope_usage(update);
                self.flush_turn_events(&mut active)?;
                self.flush_assistant_messages(&mut active, "before_usage_update")?;
                active.push_event(
                    "acp_usage_update",
                    Some(redact_json(Value::Object(update.clone()))),
                );
                Ok(())
            }
            _ => {
                self.flush_turn_events(&mut active)?;
                self.flush_assistant_messages(&mut active, "before_session_update")?;
                active.push_event(
                    &format!("acp_{update_kind}"),
                    Some(redact_json(Value::Object(update.clone()))),
                );
                Ok(())
            }
        };
        if active.pending_events.len() >= ACP_MAX_PENDING_EVENTS_PER_TURN {
            self.flush_turn_events(&mut active)?;
        }
        self.active_turns.insert(acp_session_id.to_string(), active);
        result
    }

    fn capture_session_update_without_turn(
        &self,
        acp_session_id: &str,
        update_kind: &str,
        update: &Map<String, Value>,
    ) -> Result<()> {
        let session = self
            .sessions_by_acp
            .get(acp_session_id)
            .cloned()
            .or_else(|| self.lookup_session_state(acp_session_id).ok().flatten());
        let Some(session) = session else {
            return Ok(());
        };
        if ignore_session_update(update_kind) {
            return Ok(());
        }
        let payload = if update_kind == "available_commands_update" {
            summarize_available_commands(update)
        } else {
            redact_json(Value::Object(update.clone()))
        };
        let mut db = self.open_db()?;
        db.add_lane_session_event(
            &session.lane_name,
            &session.trail_session_id,
            &format!("acp_{update_kind}"),
            Some(payload),
        )?;
        Ok(())
    }

    fn capture_tool_call_start(
        &mut self,
        active: &mut ActiveTurn,
        update: &Map<String, Value>,
    ) -> Result<()> {
        let tool_call_id = update
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let title = update
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or("ACP tool call");
        self.flush_turn_events(active)?;
        self.flush_assistant_messages(active, "before_tool_call")?;
        active.push_event(
            "tool_call",
            Some(redact_json(Value::Object(update.clone()))),
        );
        let span_id = acp_tool_span_id(active, &tool_call_id, title);
        active.push_span_started(&span_id, title, Some(Value::Object(update.clone())));
        active.tool_spans.insert(tool_call_id, span_id);
        Ok(())
    }

    fn capture_tool_call_update(
        &mut self,
        active: &mut ActiveTurn,
        update: &Map<String, Value>,
    ) -> Result<()> {
        let tool_call_id = update
            .get("toolCallId")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        let status = update
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("updated");
        self.flush_turn_events(active)?;
        self.flush_assistant_messages(active, "before_tool_call_update")?;
        active.push_event(
            "tool_call_update",
            Some(redact_json(Value::Object(update.clone()))),
        );
        if !active.materialized {
            self.flush_turn_events(active)?;
            self.capture_structured_diff_updates(active, update);
        }
        if matches!(status, "completed" | "failed" | "cancelled") {
            if let Some(span_id) = active.tool_spans.remove(&tool_call_id) {
                active.push_span_ended(&span_id, status, Some(Value::Object(update.clone())));
            }
        }
        Ok(())
    }

    fn flush_turn_events(&self, active: &mut ActiveTurn) -> Result<()> {
        if active.pending_events.is_empty() {
            return Ok(());
        }
        let events = active
            .pending_events
            .drain(..)
            .map(|event| {
                (
                    event.event_type,
                    event.payload,
                    event.change_id,
                    event.message_id,
                )
            })
            .collect::<Vec<_>>();
        let mut db = self.open_db()?;
        db.add_lane_turn_events_batch(&active.turn_id, events)?;
        Ok(())
    }

    fn flush_assistant_messages(&self, active: &mut ActiveTurn, reason: &str) -> Result<()> {
        let messages = active.drain_assistant_buffers();
        if messages.iter().all(|(_, text)| text.trim().is_empty()) {
            return Ok(());
        }
        let mut db = self.open_db()?;
        for (message_id, text) in messages {
            if text.trim().is_empty() {
                continue;
            }
            let report = db.add_lane_turn_message(&active.turn_id, "assistant", &text)?;
            db.add_lane_turn_event(
                &active.turn_id,
                "acp_agent_message_flushed",
                Some(redact_json(serde_json::json!({
                    "protocol": "acp",
                    "acp_message_id": message_id,
                    "reason": reason
                }))),
                None,
                Some(&report.message_id.0),
            )?;
        }
        Ok(())
    }

    fn capture_structured_diff_updates(
        &self,
        active: &mut ActiveTurn,
        update: &Map<String, Value>,
    ) {
        let edits = structured_diff_edits(
            update.get("content"),
            &PathBuf::from(&active.effective_cwd),
            &mut active.structured_diff_keys,
        );
        if edits.is_empty() {
            return;
        }
        let edit_count = edits.len() as u64;
        let patch = PatchDocument {
            base_change: None,
            message: Some("ACP structured diff update".to_string()),
            session_id: Some(active.trail_session_id.clone()),
            allow_ignored: false,
            allow_stale: false,
            edits,
        };
        match self
            .open_db()
            .and_then(|mut db| db.apply_lane_turn_patch(&active.turn_id, patch))
        {
            Ok(_) => {
                active.structured_diff_count += edit_count;
            }
            Err(err) => {
                eprintln!(
                    "trail acp relay capture warning: structured diff capture for lane `{}` failed: {err}",
                    active.lane_name
                );
            }
        }
    }

    fn capture_permission_request(&mut self, message: &Value) -> Result<()> {
        let Some(id) = rpc_id_key(message) else {
            return Ok(());
        };
        let params = params_object(message)?;
        let Some(acp_session_id) = params.get("sessionId").and_then(Value::as_str) else {
            return Ok(());
        };
        let options_by_id = permission_options(params.get("options"));
        let mut approval_id = None;
        if let Some(session) = self.resolve_session_state(acp_session_id).ok() {
            let active_turn_id = if let Some(mut active) = self.active_turns.remove(acp_session_id)
            {
                let turn_id = active.turn_id.clone();
                self.flush_turn_events(&mut active)?;
                self.flush_assistant_messages(&mut active, "before_permission_request")?;
                self.active_turns.insert(acp_session_id.to_string(), active);
                Some(turn_id)
            } else {
                None
            };
            let summary = params
                .get("toolCall")
                .and_then(|tool| tool.get("title"))
                .and_then(Value::as_str)
                .unwrap_or("ACP permission request");
            let report = self.open_db()?.request_lane_approval(
                &session.lane_name,
                "acp_permission",
                summary,
                Some(redact_json(Value::Object(params.clone()))),
                Some(&session.trail_session_id),
                active_turn_id.as_deref(),
            )?;
            approval_id = Some(report.approval.approval_id);
        }
        self.pending_permissions.insert(
            id,
            PendingPermission {
                approval_id,
                options_by_id,
            },
        );
        Ok(())
    }

    fn capture_cancel(&mut self, message: &Value) -> Result<()> {
        let params = params_object(message)?;
        let Some(acp_session_id) = params.get("sessionId").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some(mut active) = self.active_turns.remove(acp_session_id) else {
            return Ok(());
        };
        self.flush_turn_events(&mut active)?;
        self.flush_assistant_messages(&mut active, "before_cancel")?;
        active.push_event(
            "acp_prompt_cancel_requested",
            Some(redact_json(serde_json::json!({
                "protocol": "acp",
                "acp_session_id": acp_session_id
            }))),
        );
        self.flush_turn_events(&mut active)?;
        self.active_turns.insert(acp_session_id.to_string(), active);
        Ok(())
    }

    fn prepare_close_request(&mut self, message: &Value) -> Result<()> {
        let Some(id) = rpc_id_key(message) else {
            return Ok(());
        };
        let params = params_object(message)?;
        let Some(acp_session_id) = params.get("sessionId").and_then(Value::as_str) else {
            return Ok(());
        };
        self.pending_closes.insert(id, acp_session_id.to_string());
        Ok(())
    }

    fn finish_close_request(&mut self, message: &Value, acp_session_id: &str) -> Result<()> {
        let Some(session) = self.sessions_by_acp.get(acp_session_id).cloned() else {
            return Ok(());
        };
        let status = if message.get("error").is_some() {
            "failed"
        } else {
            "closed"
        };
        let mut db = self.open_db()?;
        db.update_lane_acp_session_status(acp_session_id, status)?;
        db.add_lane_session_event(
            &session.lane_name,
            &session.trail_session_id,
            "acp_session_closed",
            Some(redact_json(serde_json::json!({
                "protocol": "acp",
                "acp_session_id": acp_session_id,
                "status": status
            }))),
        )?;
        if status == "closed" {
            record_acp_lifecycle_event(
                &mut db,
                &session.lane_name,
                &session.trail_session_id,
                None,
                self.options.provider.as_deref(),
                acp_session_id,
                AgentLifecycleEventKind::SessionEnded,
                serde_json::json!({"status": status}),
            )?;
            let _ = db.end_lane_session(&session.trail_session_id, "completed");
            if db
                .lane_session_turns(&session.trail_session_id)?
                .iter()
                .any(|turn| turn.ended_at.is_some())
            {
                let _ = db.create_session_attestation(
                    &session.trail_session_id,
                    "acp-on-session-close",
                    Some(serde_json::json!({"acp_session_id": acp_session_id})),
                );
            }
            self.sessions_by_acp.remove(acp_session_id);
        }
        Ok(())
    }

    fn finish_open_turns(&mut self, status: &str, reason: &str) -> Result<()> {
        if self.active_turns.is_empty() {
            return Ok(());
        }
        let active_turns = std::mem::take(&mut self.active_turns);
        for (acp_session_id, mut active) in active_turns {
            let open_spans = active.tool_spans.drain().collect::<Vec<_>>();
            for (_, span_id) in open_spans {
                active.push_span_ended(&span_id, status, None);
            }
            self.flush_turn_events(&mut active)?;
            self.flush_assistant_messages(&mut active, reason)?;
            active.push_event(
                "acp_relay_turn_closed",
                Some(redact_json(serde_json::json!({
                    "protocol": "acp",
                    "acp_session_id": acp_session_id,
                    "status": status,
                    "reason": reason
                }))),
            );
            self.flush_turn_events(&mut active)?;
            let mut db = self.open_db()?;
            if active.materialized {
                let _ = db.record_lane_workdir_for_turn(
                    &active.lane_name,
                    &active.turn_id,
                    Some(format!("ACP prompt workdir checkpoint ({reason})")),
                );
            }
            db.add_lane_turn_event(
                &active.turn_id,
                "acp_prompt_finished",
                Some(redact_json(serde_json::json!({
                    "protocol": "acp",
                    "acp_session_id": acp_session_id,
                    "status": status,
                    "reason": reason
                }))),
                None,
                None,
            )?;
            if let Some(span_id) = &active.root_span_id {
                let _ = db.end_lane_trace_span(
                    span_id,
                    status,
                    Some(redact_json(serde_json::json!({ "reason": reason }))),
                );
            }
            let _ = db.end_lane_turn(&active.turn_id, status);
            let _ = db.create_turn_evidence_manifest(&active.turn_id);
            let _ = self.finalize_turn_envelope(
                &mut db,
                &active.turn_id,
                status,
                None,
                Some(summarize_text(reason)),
                Some(&active),
            );
            let _ = db.update_lane_acp_session_status(&acp_session_id, status);
            self.pending_prompts
                .retain(|_, pending| pending.acp_session_id != acp_session_id);
        }
        Ok(())
    }

    fn resolve_session_state(&mut self, acp_session_id: &str) -> Result<SessionState> {
        if let Some(session) = self.sessions_by_acp.get(acp_session_id) {
            return Ok(session.clone());
        }
        let session = self.lookup_session_state(acp_session_id)?.ok_or_else(|| {
            Error::InvalidInput(format!("ACP session `{acp_session_id}` is not mapped"))
        })?;
        self.sessions_by_acp
            .insert(acp_session_id.to_string(), session.clone());
        Ok(session)
    }

    fn lookup_session_state(&self, acp_session_id: &str) -> Result<Option<SessionState>> {
        let db = self.open_db()?;
        let Some(mapping) = db.try_lane_acp_session(acp_session_id)? else {
            return Ok(None);
        };
        let lane_name = db.resolve_lane_handle(&mapping.lane_id)?;
        Ok(Some(SessionState {
            acp_session_id: mapping.acp_session_id,
            upstream_session_id: mapping.upstream_session_id,
            lane_name,
            trail_session_id: mapping.trail_session_id,
            original_cwd: mapping.cwd.clone(),
            effective_cwd: mapping.cwd,
            materialized: false,
        }))
    }

    fn resolve_lane_name(&self, acp_session_id: Option<&str>, cwd: &str) -> String {
        if let Some(lane) = &self.options.lane {
            return lane.clone();
        }
        let provider = self
            .options
            .provider
            .as_deref()
            .or(self.options.model.as_deref())
            .unwrap_or("agent");
        let seed = format!(
            "{}:{}:{}:{}",
            provider,
            acp_session_id.unwrap_or("new"),
            cwd,
            self.options.workspace_root.display()
        );
        format!(
            "acp-{}-{}",
            sanitize_lane_component(provider),
            crate::ids::short_hash(seed.as_bytes(), 5)
        )
    }

    fn open_db(&self) -> Result<Trail> {
        Trail::open_with_db_dir(&self.options.workspace_root, &self.options.db_dir)
    }

    fn upstream_command_hash(&self) -> Option<String> {
        self.upstream_command_json
            .as_ref()
            .map(|value| crate::ids::short_hash(value.as_bytes(), 16))
    }

    fn finalize_turn_envelope(
        &self,
        db: &mut Trail,
        turn_id: &str,
        status: &str,
        stop_reason: Option<String>,
        error_summary: Option<String>,
        active: Option<&ActiveTurn>,
    ) -> Result<()> {
        let turn = db.lane_turn(turn_id)?;
        let Some(mut envelope) = TurnEnvelope::from_metadata_json(turn.metadata_json.as_deref())
        else {
            return Ok(());
        };
        if let Some(active) = active {
            envelope.usage = active.usage.clone();
            envelope.capture = TurnEnvelopeCapture {
                event_count: active.event_count,
                tool_event_count: active.tool_event_count,
                structured_diff_count: active.structured_diff_count,
                assistant_truncated: active.capture_truncated,
                redaction_applied: active.redaction_applied,
            };
        }
        let stored_events = db.lane_turn_events(turn_id)?;
        envelope.capture.event_count = stored_events.len() as u64;
        envelope.capture.tool_event_count = stored_events
            .iter()
            .filter(|event| {
                matches!(
                    event.event_type.as_str(),
                    "tool_call" | "tool_call_update" | "span_started" | "span_ended"
                )
            })
            .count() as u64;
        if stored_events
            .iter()
            .filter_map(|event| event.payload.as_ref())
            .any(value_has_redaction_marker)
        {
            envelope.capture.redaction_applied = true;
        }
        envelope.finalize_outcome(
            status.to_string(),
            stop_reason,
            &turn.before_change,
            turn.after_change.as_ref(),
            error_summary,
        );
        db.update_lane_turn_metadata(turn_id, &envelope.to_metadata_value())
    }
}

fn method_name(message: &Value) -> Option<&str> {
    message.get("method").and_then(Value::as_str)
}

fn rpc_id_key(message: &Value) -> Option<String> {
    message
        .get("id")
        .and_then(|id| serde_json::to_string(id).ok())
}

fn params_object(message: &Value) -> Result<&Map<String, Value>> {
    message
        .get("params")
        .and_then(Value::as_object)
        .ok_or_else(|| Error::InvalidInput("ACP message missing object params".to_string()))
}

fn params_object_mut(message: &mut Value) -> Result<&mut Map<String, Value>> {
    message
        .get_mut("params")
        .and_then(Value::as_object_mut)
        .ok_or_else(|| Error::InvalidInput("ACP message missing object params".to_string()))
}

fn inject_trail_mcp_server(
    params: &mut Map<String, Value>,
    options: &AcpRelayOptions,
) -> Result<()> {
    let server = trail_mcp_server(options);
    let servers = params
        .entry("mcpServers".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let servers = servers.as_array_mut().ok_or_else(|| {
        Error::InvalidInput("ACP session mcpServers must be an array".to_string())
    })?;
    let already_present = servers.iter().any(|server| {
        server
            .get("name")
            .and_then(Value::as_str)
            .is_some_and(|name| name == "trail")
    });
    if !already_present {
        servers.push(server);
    }
    Ok(())
}

fn trail_mcp_server(options: &AcpRelayOptions) -> Value {
    let command = std::env::current_exe()
        .ok()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "trail".to_string());
    serde_json::json!({
        "name": "trail",
        "command": command,
        "args": ["mcp"],
        "env": [
            {
                "name": "TRAIL_WORKSPACE",
                "value": options.workspace_root.to_string_lossy()
            },
            {
                "name": "TRAIL_DIR",
                "value": options.db_dir.to_string_lossy()
            }
        ]
    })
}

fn response_session_id(message: &Value) -> Option<&str> {
    message
        .get("result")
        .and_then(|result| result.get("sessionId"))
        .and_then(Value::as_str)
}

fn event_for_session_status(status: &str) -> &'static str {
    match status {
        "loaded" => "acp_session_loaded",
        "resumed" => "acp_session_resumed",
        "failed" => "acp_session_failed",
        _ => "acp_session_started",
    }
}

fn stop_reason(message: &Value) -> Option<&str> {
    message
        .get("result")
        .and_then(|result| result.get("stopReason"))
        .and_then(Value::as_str)
}

fn error_summary(message: &Value) -> Option<String> {
    message
        .get("error")
        .map(|error| summarize_text(&redact_json(error.clone()).to_string()))
}

fn prompt_hash(prompt: &str) -> String {
    crate::ids::short_hash(prompt.as_bytes(), 32)
}

fn turn_envelope_usage(update: &Map<String, Value>) -> TurnEnvelopeUsage {
    TurnEnvelopeUsage {
        used: usage_number_field(
            update,
            &[
                "used",
                "usedTokens",
                "used_tokens",
                "tokensUsed",
                "tokens_used",
                "totalTokens",
                "total_tokens",
            ],
        ),
        size: usage_number_field(
            update,
            &[
                "size",
                "contextSize",
                "context_size",
                "contextWindow",
                "context_window",
                "limit",
                "maxTokens",
                "max_tokens",
            ],
        ),
        cost: update.get("cost").cloned().map(redact_json),
    }
}

fn usage_number_field(update: &Map<String, Value>, keys: &[&str]) -> Option<u64> {
    keys.iter().find_map(|key| {
        update.get(*key).and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_i64().and_then(|value| u64::try_from(value).ok()))
                .or_else(|| value.as_str().and_then(|value| value.parse::<u64>().ok()))
        })
    })
}

fn prompt_status(message: &Value) -> &'static str {
    if message.get("error").is_some() {
        return "failed";
    }
    match stop_reason(message) {
        Some("cancelled") => "cancelled",
        _ => "completed",
    }
}

fn prompt_text(prompt: Option<&Value>) -> String {
    let text = match prompt {
        Some(Value::Array(blocks)) => blocks
            .iter()
            .map(block_text)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Some(value) => block_text(value),
        None => String::new(),
    };
    if text.trim().is_empty() {
        "[non-text ACP prompt content]".to_string()
    } else {
        redact_text(&text)
    }
}

fn block_text(value: &Value) -> String {
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return text.to_string();
    }
    if let Some(resource) = value.get("resource").and_then(Value::as_object) {
        let uri = resource
            .get("uri")
            .and_then(Value::as_str)
            .unwrap_or("resource");
        if let Some(text) = resource.get("text").and_then(Value::as_str) {
            return format!("[resource {uri}]\n{text}");
        }
        return format!("[resource {uri}]");
    }
    if let Some(kind) = value.get("type").and_then(Value::as_str) {
        return format!("[{kind} content]");
    }
    String::new()
}

fn content_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::Array(items)) => items
            .iter()
            .map(content_text_item)
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(""),
        Some(value) => content_text_item(value),
        None => String::new(),
    }
}

fn content_text_item(value: &Value) -> String {
    if let Some(text) = value.get("text").and_then(Value::as_str) {
        return redact_text(text);
    }
    if let Some(content) = value.get("content") {
        return content_text_item(content);
    }
    String::new()
}

fn structured_diff_edits(
    value: Option<&Value>,
    cwd: &std::path::Path,
    seen: &mut HashSet<String>,
) -> Vec<PatchEdit> {
    let mut edits = Vec::new();
    collect_structured_diff_edits(value, cwd, seen, &mut edits);
    edits
}

fn collect_structured_diff_edits(
    value: Option<&Value>,
    cwd: &std::path::Path,
    seen: &mut HashSet<String>,
    edits: &mut Vec<PatchEdit>,
) {
    let Some(value) = value else {
        return;
    };
    match value {
        Value::Array(items) => {
            for item in items {
                collect_structured_diff_edits(Some(item), cwd, seen, edits);
            }
        }
        Value::Object(object) => {
            if object
                .get("type")
                .and_then(Value::as_str)
                .is_some_and(|kind| kind == "diff")
            {
                if let Some(edit) = structured_diff_edit(object, cwd, seen) {
                    edits.push(edit);
                }
            }
            for key in ["content", "updates", "items"] {
                collect_structured_diff_edits(object.get(key), cwd, seen, edits);
            }
        }
        _ => {}
    }
}

fn structured_diff_edit(
    object: &Map<String, Value>,
    cwd: &std::path::Path,
    seen: &mut HashSet<String>,
) -> Option<PatchEdit> {
    let path = object.get("path").and_then(Value::as_str)?;
    let new_text = object
        .get("newText")
        .or_else(|| object.get("new_text"))
        .and_then(Value::as_str)?;
    let path = relative_path_from_cwd(path, cwd)?;
    let key = format!(
        "{}:{}",
        path,
        crate::ids::short_hash(new_text.as_bytes(), 16)
    );
    if !seen.insert(key) {
        return None;
    }
    Some(PatchEdit::Write {
        path,
        content: new_text.to_string(),
        executable: false,
    })
}

fn relative_path_from_cwd(path: &str, cwd: &std::path::Path) -> Option<String> {
    let path = std::path::Path::new(path);
    let relative = if path.is_absolute() {
        path.strip_prefix(cwd).ok()?
    } else {
        path
    };
    let text = relative.to_string_lossy().replace('\\', "/");
    let text = text.trim_start_matches("./");
    if text.is_empty() || text.starts_with("../") || text == ".." {
        None
    } else {
        Some(text.to_string())
    }
}

fn permission_options(value: Option<&Value>) -> HashMap<String, String> {
    let mut options = HashMap::new();
    let Some(Value::Array(items)) = value else {
        return options;
    };
    for item in items {
        let Some(option_id) = item.get("optionId").and_then(Value::as_str) else {
            continue;
        };
        let kind = item
            .get("kind")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();
        options.insert(option_id.to_string(), kind);
    }
    options
}

fn permission_decision(message: &Value, options_by_id: &HashMap<String, String>) -> &'static str {
    let Some(outcome) = message
        .get("result")
        .and_then(|result| result.get("outcome"))
        .and_then(Value::as_object)
    else {
        return "cancelled";
    };
    if outcome
        .get("outcome")
        .and_then(Value::as_str)
        .is_some_and(|value| value == "cancelled")
    {
        return "cancelled";
    }
    let Some(option_id) = outcome.get("optionId").and_then(Value::as_str) else {
        return "cancelled";
    };
    match options_by_id.get(option_id).map(String::as_str) {
        Some("allow_once" | "allow_always") => "approved",
        Some("reject_once" | "reject_always") => "rejected",
        _ => "cancelled",
    }
}

fn sanitize_lane_component(value: &str) -> String {
    let mut out = String::new();
    for ch in value.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if matches!(ch, '-' | '_') && !out.ends_with('-') {
            out.push('-');
        }
    }
    let out = out.trim_matches('-');
    if out.is_empty() {
        "agent".to_string()
    } else {
        out.chars().take(24).collect()
    }
}

fn summarize_text(text: &str) -> String {
    const LIMIT: usize = 512;
    if text.len() <= LIMIT {
        return text.to_string();
    }
    let mut summary = text.chars().take(LIMIT).collect::<String>();
    summary.push_str("...");
    summary
}

fn utf8_prefix_len(value: &str, max_bytes: usize) -> usize {
    if value.len() <= max_bytes {
        return value.len();
    }
    let mut end = max_bytes;
    while end > 0 && !value.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn acp_trace_id_for_turn(turn_id: &str) -> String {
    format!("trace_{}", crate::ids::short_hash(turn_id.as_bytes(), 16))
}

fn acp_tool_span_id(active: &ActiveTurn, tool_call_id: &str, title: &str) -> String {
    let seed = format!(
        "{}:{}:{}:{}:{}",
        active.lane_name,
        active.turn_id,
        tool_call_id,
        title,
        active.tool_spans.len()
    );
    format!("span_{}", crate::ids::short_hash(seed.as_bytes(), 16))
}

fn summarize_available_commands(update: &Map<String, Value>) -> Value {
    let commands = update
        .get("commands")
        .or_else(|| update.get("availableCommands"))
        .or_else(|| update.get("available_commands"))
        .and_then(Value::as_array);
    let names = commands
        .map(|commands| {
            commands
                .iter()
                .filter_map(command_display_name)
                .take(100)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let total = commands.map(Vec::len).unwrap_or(0);
    redact_json(serde_json::json!({
        "protocol": "acp",
        "sessionUpdate": "available_commands_update",
        "command_count": total,
        "command_names": names,
        "truncated": total > 100
    }))
}

fn ignore_session_update(update_kind: &str) -> bool {
    update_kind == "agent_thought_chunk"
}

fn command_display_name(value: &Value) -> Option<String> {
    for key in ["name", "title", "command", "commandId", "id"] {
        if let Some(text) = value.get(key).and_then(Value::as_str) {
            return Some(text.to_string());
        }
    }
    None
}

fn redact_command(command: &[String]) -> Vec<String> {
    let mut redacted = Vec::with_capacity(command.len());
    let mut redact_next = false;
    for arg in command {
        if redact_next {
            redacted.push("<redacted>".to_string());
            redact_next = false;
            continue;
        }
        let lower = arg.to_ascii_lowercase();
        if lower.contains("token")
            || lower.contains("secret")
            || lower.contains("password")
            || lower.contains("api-key")
            || lower.contains("apikey")
        {
            redacted.push(redact_arg_value(arg));
            if !arg.contains('=') {
                redact_next = true;
            }
        } else {
            redacted.push(arg.clone());
        }
    }
    redacted
}

fn redact_arg_value(arg: &str) -> String {
    if let Some((key, _)) = arg.split_once('=') {
        format!("{key}=<redacted>")
    } else {
        arg.to_string()
    }
}

fn redact_json(value: Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut redacted = Map::new();
            for (key, value) in object {
                if secret_key(&key) {
                    redacted.insert(key, Value::String("<redacted>".to_string()));
                } else {
                    redacted.insert(key, redact_json(value));
                }
            }
            Value::Object(redacted)
        }
        Value::Array(items) => Value::Array(items.into_iter().map(redact_json).collect()),
        Value::String(text) => Value::String(redact_text(&text)),
        other => other,
    }
}

fn value_has_redaction_marker(value: &Value) -> bool {
    match value {
        Value::String(text) => text.contains("<redacted>") || text.contains("[REDACTED]"),
        Value::Array(items) => items.iter().any(value_has_redaction_marker),
        Value::Object(object) => object.values().any(value_has_redaction_marker),
        _ => false,
    }
}

fn secret_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase();
    lower.contains("token")
        || lower.contains("secret")
        || lower.contains("password")
        || lower.contains("credential")
        || lower == "authorization"
        || lower == "api_key"
        || lower == "apikey"
}

fn redact_text(text: &str) -> String {
    const LIMIT: usize = 64 * 1024;
    let mut out = if text.len() > LIMIT {
        let mut truncated = text.chars().take(LIMIT).collect::<String>();
        truncated.push_str("\n[truncated]");
        truncated
    } else {
        text.to_string()
    };
    for marker in ["Authorization:", "authorization:", "Bearer "] {
        if let Some(index) = out.find(marker) {
            let end = out[index..]
                .find('\n')
                .map(|offset| index + offset)
                .unwrap_or(out.len());
            out.replace_range(index..end, "<redacted>");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::InitImportMode;
    use std::fs;

    #[test]
    fn raw_json_frame_preserves_a_single_message() {
        let raw = br#" {"jsonrpc":"2.0","id":1,"method":"initialize"}
"#
        .to_vec();
        let frame = Frame::parse(Direction::ClientToAgent, raw.clone()).unwrap();
        assert_eq!(frame.value()["method"], "initialize");
        assert_eq!(frame.forward_bytes(), raw);
    }

    #[test]
    fn injects_trail_mcp_server_once() {
        let options = AcpRelayOptions {
            workspace_root: PathBuf::from("/tmp/workspace"),
            db_dir: PathBuf::from("/tmp/workspace/.trail"),
            lane: None,
            from_ref: None,
            provider: None,
            model: None,
            materialize: false,
            workdir: None,
            inject_mcp: true,
            upstream_command: vec!["agent".to_string()],
            upstream_env: BTreeMap::new(),
        };
        let mut params = Map::new();
        params.insert("mcpServers".to_string(), Value::Array(Vec::new()));
        inject_trail_mcp_server(&mut params, &options).unwrap();
        inject_trail_mcp_server(&mut params, &options).unwrap();
        let servers = params["mcpServers"].as_array().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["name"], "trail");
        assert_eq!(servers[0]["args"], serde_json::json!(["mcp"]));
    }

    #[test]
    fn provider_profiles_cover_acp_and_terminal_modes() {
        let cursor = acp_provider_profile("cursor").unwrap();
        assert_eq!(cursor.agent, "cursor");
        assert!(cursor.supports_acp);
        assert!(cursor.supports_mcp);
        assert!(cursor.supports_terminal);
        assert_eq!(
            cursor.relay_command,
            [
                "trail".to_string(),
                "acp".to_string(),
                "relay".to_string(),
                "cursor".to_string()
            ]
        );
        assert_eq!(
            acp_provider_upstream_command("cursor").unwrap(),
            ["agent".to_string(), "acp".to_string()]
        );
        assert_eq!(
            acp_provider_upstream_command("codex-cli").unwrap(),
            [
                "npx".to_string(),
                "-y".to_string(),
                CODEX_ACP_ADAPTER.to_string()
            ]
        );

        let gemini = agent_provider_profile("gemini-cli").unwrap();
        assert_eq!(gemini.agent, "gemini");
        assert!(!gemini.supports_acp);
        assert!(gemini.supports_mcp);
        assert!(gemini.supports_terminal);
        assert!(gemini.relay_command.is_empty());
        assert_eq!(
            terminal_agent_command("opencode").unwrap(),
            vec!["opencode".to_string()]
        );
    }

    #[test]
    fn prompt_text_extracts_text_and_resource_content() {
        let prompt = serde_json::json!([
            {"type": "text", "text": "hello"},
            {"type": "resource", "resource": {"uri": "file:///a.rs", "text": "fn main() {}"}}
        ]);
        let text = prompt_text(Some(&prompt));
        assert!(text.contains("hello"));
        assert!(text.contains("file:///a.rs"));
        assert!(text.contains("fn main()"));
    }

    #[test]
    fn redacts_secret_json_fields() {
        let value = redact_json(serde_json::json!({
            "token": "abc",
            "nested": { "password": "def" },
            "safe": "value"
        }));
        assert_eq!(value["token"], "<redacted>");
        assert_eq!(value["nested"]["password"], "<redacted>");
        assert_eq!(value["safe"], "value");
    }

    #[test]
    fn structured_diff_edits_accept_paths_under_cwd_once() {
        let cwd = PathBuf::from("/repo");
        let update = serde_json::json!([
            {"type": "diff", "path": "/repo/src/lib.rs", "newText": "pub fn x() {}\n"},
            {"type": "diff", "path": "/repo/src/lib.rs", "newText": "pub fn x() {}\n"},
            {"type": "diff", "path": "/other/src/lib.rs", "newText": "bad\n"}
        ]);
        let mut seen = HashSet::new();
        let edits = structured_diff_edits(Some(&update), &cwd, &mut seen);
        assert_eq!(edits.len(), 1);
        match &edits[0] {
            PatchEdit::Write { path, content, .. } => {
                assert_eq!(path, "src/lib.rs");
                assert_eq!(content, "pub fn x() {}\n");
            }
            other => panic!("unexpected edit: {other:?}"),
        }
    }

    #[test]
    fn available_commands_summary_omits_descriptions() {
        let update = serde_json::json!({
            "sessionUpdate": "available_commands_update",
            "commands": [
                {"name": "apply_patch", "description": "large sensitive prose"},
                {"title": "Run command", "input": {"token": "secret"}}
            ]
        });
        let summary = summarize_available_commands(update.as_object().unwrap());
        assert_eq!(summary["sessionUpdate"], "available_commands_update");
        assert_eq!(summary["command_count"], 2);
        assert_eq!(
            summary["command_names"],
            serde_json::json!(["apply_patch", "Run command"])
        );
        assert!(summary.get("description").is_none());
        assert!(!summary.to_string().contains("large sensitive prose"));
        assert!(!summary.to_string().contains("secret"));
    }

    #[test]
    fn agent_thought_chunks_are_not_captured() {
        assert!(ignore_session_update("agent_thought_chunk"));
        assert!(!ignore_session_update("agent_message_chunk"));
        assert!(!ignore_session_update("tool_call"));
    }

    #[test]
    fn turn_envelope_legacy_metadata_returns_none() {
        let metadata = serde_json::json!({
            "kind": "acp_prompt",
            "protocol": "acp"
        })
        .to_string();
        assert!(TurnEnvelope::from_metadata_json(Some(&metadata)).is_none());
    }

    #[test]
    fn turn_envelope_usage_update_captured() {
        let update = serde_json::json!({
            "sessionUpdate": "usage_update",
            "used": 42,
            "size": "100",
            "cost": {
                "usd": "0.01",
                "token": "secret"
            }
        });
        let usage = turn_envelope_usage(update.as_object().unwrap());
        assert_eq!(usage.used, Some(42));
        assert_eq!(usage.size, Some(100));
        assert_eq!(usage.cost.unwrap()["token"], "<redacted>");
    }

    #[test]
    fn turn_envelope_acp_prompt_finish_checkpoint() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db_dir = temp.path().join(".trail");
        let cwd = temp.path().to_string_lossy().to_string();
        let mut coordinator = CaptureCoordinator::new(AcpRelayOptions {
            workspace_root: temp.path().to_path_buf(),
            db_dir,
            lane: Some("agent-test".to_string()),
            from_ref: None,
            provider: Some("codex".to_string()),
            model: Some("gpt-test".to_string()),
            materialize: false,
            workdir: None,
            inject_mcp: false,
            upstream_command: vec![
                "codex".to_string(),
                "--api-key".to_string(),
                "secret".to_string(),
            ],
            upstream_env: BTreeMap::new(),
        })
        .unwrap();

        let mut session_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "session/new",
            "params": {
                "sessionId": "client-session",
                "cwd": cwd
            }
        });
        coordinator
            .before_client_message(&mut session_request)
            .unwrap();
        let mut session_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "sessionId": "upstream-session"
            }
        });
        coordinator
            .before_agent_message(&mut session_response)
            .unwrap();

        let mut prompt_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "session/prompt",
            "params": {
                "sessionId": "upstream-session",
                "prompt": [
                    { "type": "text", "text": "Write src/lib.rs" }
                ]
            }
        });
        coordinator
            .before_client_message(&mut prompt_request)
            .unwrap();
        let mut usage_update = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "upstream-session",
                "update": {
                    "sessionUpdate": "usage_update",
                    "used": 42,
                    "size": 100
                }
            }
        });
        coordinator.before_agent_message(&mut usage_update).unwrap();
        let mut diff_update = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "session/update",
            "params": {
                "sessionId": "upstream-session",
                "update": {
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": "edit-1",
                    "title": "Edit src/lib.rs",
                    "status": "completed",
                    "content": [
                        {
                            "type": "diff",
                            "path": "src/lib.rs",
                            "newText": "pub fn generated() {}\n"
                        }
                    ]
                }
            }
        });
        coordinator.before_agent_message(&mut diff_update).unwrap();
        let mut prompt_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "stopReason": "end_turn"
            }
        });
        coordinator
            .before_agent_message(&mut prompt_response)
            .unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let transcript = db.transcript("agent-test").unwrap();
        assert_eq!(transcript.turns.len(), 1);
        let turn = &transcript.turns[0];
        let envelope = turn.turn_envelope.as_ref().unwrap();
        assert_eq!(envelope.schema, TURN_ENVELOPE_SCHEMA);
        assert_eq!(envelope.version, TURN_ENVELOPE_VERSION);
        assert_eq!(envelope.provider.as_deref(), Some("codex"));
        assert_eq!(envelope.model.as_deref(), Some("gpt-test"));
        assert_eq!(
            envelope.session.upstream_session_id.as_deref(),
            Some("upstream-session")
        );
        assert!(envelope.prompt.hash.is_some());
        assert!(envelope.prompt.user_message_id.is_some());
        assert_eq!(envelope.usage.used, Some(42));
        assert_eq!(envelope.usage.size, Some(100));
        assert_eq!(envelope.capture.structured_diff_count, 1);
        assert!(envelope.capture.event_count > 0);
        assert!(envelope.capture.tool_event_count > 0);
        assert!(!envelope.outcome.no_changes);
        assert!(envelope.outcome.checkpoint.is_some());
        assert_eq!(turn.checkpoint, envelope.outcome.checkpoint);

        let report = db.agent_turn("agent-test", "1", None, false).unwrap();
        assert_eq!(
            report.turn_envelope.as_ref().unwrap().outcome.checkpoint,
            envelope.outcome.checkpoint
        );
    }

    #[test]
    fn turn_envelope_acp_prompt_finish_no_changes() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let cwd = temp.path().to_string_lossy().to_string();
        let mut coordinator = CaptureCoordinator::new(AcpRelayOptions {
            workspace_root: temp.path().to_path_buf(),
            db_dir: temp.path().join(".trail"),
            lane: Some("agent-no-change".to_string()),
            from_ref: None,
            provider: Some("codex".to_string()),
            model: Some("gpt-test".to_string()),
            materialize: false,
            workdir: None,
            inject_mcp: false,
            upstream_command: vec!["codex".to_string()],
            upstream_env: BTreeMap::new(),
        })
        .unwrap();

        let mut session_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "session/new",
            "params": {
                "sessionId": "client-session",
                "cwd": cwd
            }
        });
        coordinator
            .before_client_message(&mut session_request)
            .unwrap();
        let mut session_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": {
                "sessionId": "upstream-session"
            }
        });
        coordinator
            .before_agent_message(&mut session_response)
            .unwrap();
        let mut prompt_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "session/prompt",
            "params": {
                "sessionId": "upstream-session",
                "prompt": [
                    { "type": "text", "text": "Inspect only" }
                ]
            }
        });
        coordinator
            .before_client_message(&mut prompt_request)
            .unwrap();
        let mut prompt_response = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "result": {
                "stopReason": "end_turn"
            }
        });
        coordinator
            .before_agent_message(&mut prompt_response)
            .unwrap();

        let db = Trail::open(temp.path()).unwrap();
        let transcript = db.transcript("agent-no-change").unwrap();
        let turn = &transcript.turns[0];
        let envelope = turn.turn_envelope.as_ref().unwrap();
        assert!(envelope.outcome.no_changes);
        assert!(envelope.outcome.checkpoint.is_none());
        assert!(turn.checkpoint.is_none());
    }
}
