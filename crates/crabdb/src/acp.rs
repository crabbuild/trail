use std::collections::{HashMap, HashSet};
use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::model::*;
use crate::{CrabDb, Error, PatchDocument, PatchEdit, Result};

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
}

pub fn acp_provider_profile(agent: &str) -> Result<AcpProviderProfile> {
    match agent {
        "claude-code" | "claude" => {
            let relay_command = npx_adapter_relay_command("claude-code", CLAUDE_ACP_ADAPTER);
            let npx_available = command_in_path("npx");
            Ok(AcpProviderProfile {
                agent: "claude-code".to_string(),
                display_name: "Claude Code".to_string(),
                available: npx_available,
                relay_command,
                notes: if npx_available {
                    vec!["uses the Claude ACP adapter through npx".to_string()]
                } else {
                    vec!["`npx` was not found on PATH".to_string()]
                },
            })
        }
        "codex" | "codex-cli" | "openai-codex" => {
            let relay_command = npx_adapter_relay_command("codex", CODEX_ACP_ADAPTER);
            let npx_available = command_in_path("npx");
            Ok(AcpProviderProfile {
                agent: "codex".to_string(),
                display_name: "Codex".to_string(),
                available: npx_available,
                relay_command,
                notes: if npx_available {
                    vec!["uses the Codex ACP adapter through npx".to_string()]
                } else {
                    vec!["`npx` was not found on PATH".to_string()]
                },
            })
        }
        other => Err(Error::InvalidInput(format!(
            "unsupported ACP agent `{other}`; supported agents: {}; use `crabdb acp relay -- <COMMAND>...` for another ACP-compatible agent",
            supported_acp_agents().join(", ")
        ))),
    }
}

pub fn acp_provider_profiles() -> Vec<AcpProviderProfile> {
    supported_acp_agents()
        .into_iter()
        .filter_map(|agent| acp_provider_profile(agent).ok())
        .collect()
}

pub fn acp_install_report(agent: &str, editor: &str, dry_run: bool) -> Result<AcpInstallReport> {
    let profile = acp_provider_profile(agent)?;
    let editor = match editor {
        "generic" | "zed" => editor,
        other => {
            return Err(Error::InvalidInput(format!(
                "unsupported ACP editor `{other}`; supported editors: generic, zed"
            )))
        }
    };
    let snippet = acp_editor_snippet(editor, &profile.agent, &profile.relay_command);
    Ok(AcpInstallReport {
        agent: profile.agent,
        editor: editor.to_string(),
        dry_run,
        relay_command: profile.relay_command,
        snippet,
        detected: profile.available,
        warnings: if profile.available {
            Vec::new()
        } else {
            profile.notes
        },
    })
}

fn supported_acp_agents() -> Vec<&'static str> {
    vec!["claude-code", "codex"]
}

fn npx_adapter_relay_command(provider: &str, adapter: &str) -> Vec<String> {
    vec![
        "crabdb".to_string(),
        "acp".to_string(),
        "relay".to_string(),
        "--provider".to_string(),
        provider.to_string(),
        "--materialize".to_string(),
        "--".to_string(),
        "npx".to_string(),
        "-y".to_string(),
        adapter.to_string(),
    ]
}

pub fn run_stdio_relay(options: AcpRelayOptions) -> Result<()> {
    if options.upstream_command.is_empty() {
        return Err(Error::InvalidInput(
            "ACP relay requires an upstream command after `--`".to_string(),
        ));
    }

    let mut child = Command::new(&options.upstream_command[0])
        .args(&options.upstream_command[1..])
        .current_dir(&options.workspace_root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|err| {
            Error::InvalidInput(format!(
                "failed to launch upstream ACP agent `{}`: {err}",
                options.upstream_command[0]
            ))
        })?;

    let child_stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::InvalidInput("failed to open upstream ACP stdin pipe".to_string()))?;
    let child_stdout = child.stdout.take().ok_or_else(|| {
        Error::InvalidInput("failed to open upstream ACP stdout pipe".to_string())
    })?;

    if let Some(stderr) = child.stderr.take() {
        thread::spawn(move || {
            let _ = copy_upstream_stderr(stderr);
        });
    }

    let coordinator = Arc::new(Mutex::new(CaptureCoordinator::new(options)?));
    let (done_tx, done_rx) = mpsc::channel();

    let editor_coordinator = Arc::clone(&coordinator);
    let editor_done = done_tx.clone();
    let editor_handle = thread::spawn(move || {
        let result = pump_editor_to_agent(io::stdin().lock(), child_stdin, editor_coordinator);
        let _ = editor_done.send(PumpDone::Editor(result));
    });

    let agent_coordinator = Arc::clone(&coordinator);
    let agent_handle = thread::spawn(move || {
        let result = pump_agent_to_editor(
            BufReader::new(child_stdout),
            io::stdout(),
            agent_coordinator,
        );
        let _ = done_tx.send(PumpDone::Agent(result));
    });

    let first = done_rx.recv().map_err(|err| {
        Error::InvalidInput(format!("ACP relay pump failed before startup: {err}"))
    })?;
    match first {
        PumpDone::Editor(result) => {
            result.map_err(Error::Io)?;
            let status = child.wait().map_err(Error::Io)?;
            if let Ok(PumpDone::Agent(result)) = done_rx.recv() {
                result.map_err(Error::Io)?;
            }
            let _ = agent_handle.join();
            let _ = editor_handle.join();
            if status.success() {
                Ok(())
            } else {
                Err(Error::InvalidInput(format!(
                    "upstream ACP agent exited with status {status}"
                )))
            }
        }
        PumpDone::Agent(result) => {
            result.map_err(Error::Io)?;
            let status = child.wait().map_err(Error::Io)?;
            if status.success() {
                Ok(())
            } else {
                Err(Error::InvalidInput(format!(
                    "upstream ACP agent exited with status {status}"
                )))
            }
        }
    }
}

fn command_in_path(command: &str) -> bool {
    if command.contains(std::path::MAIN_SEPARATOR) {
        return PathBuf::from(command).is_file();
    }
    let Some(path) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&path).any(|dir| dir.join(command).is_file())
}

fn acp_editor_snippet(editor: &str, agent: &str, relay_command: &[String]) -> String {
    let command = shell_join(relay_command);
    match editor {
        "zed" => serde_json::to_string_pretty(&serde_json::json!({
            "agent_servers": {
                (format!("crabdb-{agent}")): {
                    "type": "custom",
                    "command": relay_command.first().cloned().unwrap_or_default(),
                    "args": relay_command.iter().skip(1).cloned().collect::<Vec<_>>()
                }
            }
        }))
        .unwrap_or_else(|_| "{}".to_string()),
        _ => format!("ACP command:\n{command}"),
    }
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

enum PumpDone {
    Editor(io::Result<()>),
    Agent(io::Result<()>),
}

fn pump_editor_to_agent<R, W>(
    mut reader: R,
    mut writer: W,
    coordinator: Arc<Mutex<CaptureCoordinator>>,
) -> io::Result<()>
where
    R: BufRead,
    W: Write,
{
    loop {
        let mut message = match read_json_line(&mut reader) {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(err) => {
                capture_step(&coordinator, |capture| {
                    capture.finish_open_turns("failed", "editor sent malformed JSON")
                });
                return Err(err);
            }
        };
        capture_step(&coordinator, |capture| {
            capture.before_client_message(&mut message)
        });
        write_json_line(&mut writer, &message)?;
    }
    capture_step(&coordinator, |capture| {
        capture.finish_open_turns("cancelled", "editor input closed")
    });
    writer.flush()
}

fn pump_agent_to_editor<R, W>(
    mut reader: R,
    mut writer: W,
    coordinator: Arc<Mutex<CaptureCoordinator>>,
) -> io::Result<()>
where
    R: BufRead,
    W: Write,
{
    loop {
        let mut message = match read_json_line(&mut reader) {
            Ok(Some(message)) => message,
            Ok(None) => break,
            Err(err) => {
                capture_step(&coordinator, |capture| {
                    capture.finish_open_turns("failed", "upstream sent malformed JSON")
                });
                return Err(err);
            }
        };
        capture_step(&coordinator, |capture| {
            capture.before_agent_message(&mut message)
        });
        write_json_line(&mut writer, &message)?;
    }
    capture_step(&coordinator, |capture| {
        capture.finish_open_turns("failed", "upstream output closed")
    });
    writer.flush()
}

fn capture_step<F>(coordinator: &Arc<Mutex<CaptureCoordinator>>, f: F)
where
    F: FnOnce(&mut CaptureCoordinator) -> Result<()>,
{
    match coordinator.lock() {
        Ok(mut capture) => {
            let result = CrabDb::with_write_lock_wait(ACP_CAPTURE_LOCK_WAIT, || f(&mut capture));
            if let Err(err) = result {
                eprintln!("crabdb acp relay capture warning: {err}");
            }
        }
        Err(_) => {
            eprintln!("crabdb acp relay capture warning: capture coordinator lock poisoned");
        }
    }
}

fn read_json_line<R: BufRead>(reader: &mut R) -> io::Result<Option<Value>> {
    loop {
        let mut line = String::new();
        let bytes = reader.read_line(&mut line)?;
        if bytes == 0 {
            return Ok(None);
        }
        let line = line.trim_end_matches(['\r', '\n']);
        if line.trim().is_empty() {
            continue;
        }
        let value = serde_json::from_str(line)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        return Ok(Some(value));
    }
}

fn write_json_line<W: Write>(writer: &mut W, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, value)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
    writer.write_all(b"\n")?;
    writer.flush()
}

fn copy_upstream_stderr<R: Read>(reader: R) -> io::Result<()> {
    let mut reader = BufReader::new(reader);
    let mut buf = [0u8; 8192];
    loop {
        let bytes = reader.read(&mut buf)?;
        if bytes == 0 {
            return Ok(());
        }
        let mut stderr = io::stderr().lock();
        stderr.write_all(&buf[..bytes])?;
        stderr.flush()?;
    }
}

#[derive(Clone, Debug)]
struct SessionState {
    acp_session_id: String,
    lane_name: String,
    crabdb_session_id: String,
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
    crabdb_session_id: String,
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
        self.pending_events.push(BufferedTurnEvent {
            event_type: event_type.into(),
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
    pending_initialize: HashSet<String>,
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
            pending_initialize: HashSet::new(),
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
            Some("initialize") => {
                if let Some(id) = rpc_id_key(message) {
                    self.pending_initialize.insert(id);
                }
            }
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

        if self.pending_initialize.remove(&id) {
            self.add_initialize_metadata(message);
            return Ok(());
        }

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

    fn add_initialize_metadata(&self, message: &mut Value) {
        let Some(result) = message.get_mut("result").and_then(Value::as_object_mut) else {
            return;
        };
        let meta = ensure_object_field(result, "_meta");
        meta.insert(
            "crabdb".to_string(),
            serde_json::json!({
                "relay": true,
                "capture": true,
                "workspace": self.options.workspace_root.to_string_lossy(),
                "dbDir": self.options.db_dir.to_string_lossy(),
                "provider": self.options.provider,
                "model": self.options.model
            }),
        );
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
            inject_crabdb_mcp_server(params, &self.options)?;
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
                    lane_name,
                    crabdb_session_id: mapping.crabdb_session_id,
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
            lane_name,
            crabdb_session_id: session.session_id,
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

        let mut db = self.open_db()?;
        db.upsert_lane_acp_session(
            &acp_session_id,
            response_session_id(message),
            &session.lane_name,
            &session.crabdb_session_id,
            &session.effective_cwd,
            self.options.provider.as_deref(),
            self.options.model.as_deref(),
            self.upstream_command_json.as_deref(),
            status,
        )?;
        db.add_lane_session_event(
            &session.lane_name,
            &session.crabdb_session_id,
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
        if status == "failed" {
            let _ = db.end_lane_session(&session.crabdb_session_id, "failed");
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
        let turn = db.begin_lane_session_turn(
            &session.lane_name,
            &session.crabdb_session_id,
            Some(redact_json(serde_json::json!({
                "kind": "acp_prompt",
                "protocol": "acp",
                "acp_session_id": acp_session_id,
                "provider": self.options.provider,
                "model": self.options.model,
                "cwd": session.original_cwd,
                "effective_cwd": session.effective_cwd,
                "upstream_command_hash": self.upstream_command_hash()
            }))),
        )?;
        db.add_lane_turn_message(&turn.turn.turn_id, "user", &prompt_text)?;
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
                crabdb_session_id: session.crabdb_session_id,
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
            &session.crabdb_session_id,
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
        let patch = PatchDocument {
            base_change: None,
            message: Some("ACP structured diff update".to_string()),
            session_id: Some(active.crabdb_session_id.clone()),
            allow_ignored: false,
            allow_stale: false,
            edits,
        };
        match self
            .open_db()
            .and_then(|mut db| db.apply_lane_turn_patch(&active.turn_id, patch))
        {
            Ok(_) => {}
            Err(err) => {
                eprintln!(
                    "crabdb acp relay capture warning: structured diff capture for lane `{}` failed: {err}",
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
                Some(&session.crabdb_session_id),
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
            &session.crabdb_session_id,
            "acp_session_closed",
            Some(redact_json(serde_json::json!({
                "protocol": "acp",
                "acp_session_id": acp_session_id,
                "status": status
            }))),
        )?;
        if status == "closed" {
            let _ = db.end_lane_session(&session.crabdb_session_id, "completed");
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
            if let Some(span_id) = active.root_span_id {
                let _ = db.end_lane_trace_span(
                    &span_id,
                    status,
                    Some(redact_json(serde_json::json!({ "reason": reason }))),
                );
            }
            let _ = db.end_lane_turn(&active.turn_id, status);
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
            lane_name,
            crabdb_session_id: mapping.crabdb_session_id,
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

    fn open_db(&self) -> Result<CrabDb> {
        CrabDb::open_with_db_dir(&self.options.workspace_root, &self.options.db_dir)
    }

    fn upstream_command_hash(&self) -> Option<String> {
        self.upstream_command_json
            .as_ref()
            .map(|value| crate::ids::short_hash(value.as_bytes(), 16))
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

fn ensure_object_field<'a>(
    object: &'a mut Map<String, Value>,
    key: &str,
) -> &'a mut Map<String, Value> {
    let value = object
        .entry(key.to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if !value.is_object() {
        *value = Value::Object(Map::new());
    }
    value.as_object_mut().expect("object was just inserted")
}

fn inject_crabdb_mcp_server(
    params: &mut Map<String, Value>,
    options: &AcpRelayOptions,
) -> Result<()> {
    let server = crabdb_mcp_server(options);
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
            .is_some_and(|name| name == "crabdb")
    });
    if !already_present {
        servers.push(server);
    }
    Ok(())
}

fn crabdb_mcp_server(options: &AcpRelayOptions) -> Value {
    let command = std::env::current_exe()
        .ok()
        .map(|path| path.to_string_lossy().to_string())
        .unwrap_or_else(|| "crabdb".to_string());
    serde_json::json!({
        "name": "crabdb",
        "command": command,
        "args": ["mcp"],
        "env": [
            {
                "name": "CRABDB_WORKSPACE",
                "value": options.workspace_root.to_string_lossy()
            },
            {
                "name": "CRABDB_DIR",
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

    #[test]
    fn json_line_round_trips_single_message() {
        let mut input =
            BufReader::new(br#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#.as_slice());
        let value = read_json_line(&mut input).unwrap().unwrap();
        assert_eq!(value["method"], "initialize");

        let mut out = Vec::new();
        write_json_line(&mut out, &value).unwrap();
        assert!(std::str::from_utf8(&out).unwrap().ends_with('\n'));
    }

    #[test]
    fn injects_crabdb_mcp_server_once() {
        let options = AcpRelayOptions {
            workspace_root: PathBuf::from("/tmp/workspace"),
            db_dir: PathBuf::from("/tmp/workspace/.crabdb"),
            lane: None,
            from_ref: None,
            provider: None,
            model: None,
            materialize: false,
            workdir: None,
            inject_mcp: true,
            upstream_command: vec!["agent".to_string()],
        };
        let mut params = Map::new();
        params.insert("mcpServers".to_string(), Value::Array(Vec::new()));
        inject_crabdb_mcp_server(&mut params, &options).unwrap();
        inject_crabdb_mcp_server(&mut params, &options).unwrap();
        let servers = params["mcpServers"].as_array().unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0]["name"], "crabdb");
        assert_eq!(servers[0]["args"], serde_json::json!(["mcp"]));
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
}
