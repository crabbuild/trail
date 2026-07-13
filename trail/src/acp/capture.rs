use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::protocol::Direction;
use super::transport::RelayFinishReason;
use super::CaptureCoordinator;
use crate::model::{AgentCaptureTransport, AgentHookReceiptInput};
use crate::{Error, Result, Trail};

const CAPTURE_QUEUE_CAPACITY: usize = 4_096;
const CAPTURE_MESSAGE_LIMIT: usize = 512 * 1024;
const CAPTURE_PROJECT_LOCK_WAIT: Duration = Duration::from_millis(250);
const CAPTURE_RETRY_INTERVAL: Duration = Duration::from_millis(25);
const CAPTURE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const CAPTURE_SHUTDOWN_DRAIN_BUDGET: Duration = Duration::from_millis(500);
static SPILL_CLAIM_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct CapturedFrame {
    pub connection_id: String,
    pub direction: Direction,
    pub sequence: u64,
    pub received_at: i64,
    pub redacted_message: Value,
    pub project: bool,
}

pub(crate) enum CaptureCommand {
    Frame(CapturedFrame),
    Finish(RelayFinishReason),
    Barrier(mpsc::Sender<()>),
}

#[derive(Default)]
pub(crate) struct CaptureHealth {
    healthy: AtomicBool,
    degraded: AtomicBool,
    last_error: Mutex<Option<String>>,
    queued: AtomicUsize,
    spilled: AtomicUsize,
    last_projected_sequence: AtomicU64,
}

impl CaptureHealth {
    fn new() -> Self {
        Self {
            healthy: AtomicBool::new(true),
            ..Self::default()
        }
    }

    fn record_error(&self, error: &Error) -> bool {
        self.healthy.store(false, Ordering::Release);
        let first = !self.degraded.swap(true, Ordering::AcqRel);
        if let Ok(mut last_error) = self.last_error.lock() {
            *last_error = Some(error.to_string());
        }
        first
    }
}

#[allow(dead_code)]
pub(crate) struct CaptureShutdownReport {
    pub healthy: bool,
    pub degraded: bool,
    pub queued: usize,
    pub spilled: usize,
    pub last_projected_sequence: u64,
}

pub(crate) struct CaptureIngress {
    tx: Option<mpsc::SyncSender<CaptureCommand>>,
    health: Arc<CaptureHealth>,
    worker: Option<thread::JoinHandle<()>>,
    spill: Arc<SpillStore>,
    spill_mode: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    pending_finish: Arc<Mutex<Option<RelayFinishReason>>>,
    done_rx: Mutex<mpsc::Receiver<()>>,
}

impl CaptureIngress {
    pub(crate) fn new(
        workspace_root: PathBuf,
        db_dir: PathBuf,
        coordinator: Arc<Mutex<CaptureCoordinator>>,
        _connection_id: String,
    ) -> Result<Self> {
        let spill = Arc::new(SpillStore::new(db_dir.join("acp-ingress"))?);
        let health = Arc::new(CaptureHealth::new());
        let spill_mode = Arc::new(AtomicBool::new(false));
        let stopping = Arc::new(AtomicBool::new(false));
        let pending_finish = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::sync_channel(CAPTURE_QUEUE_CAPACITY);
        let (done_tx, done_rx) = mpsc::channel();
        let worker_health = Arc::clone(&health);
        let worker_spill = Arc::clone(&spill);
        let worker_spill_mode = Arc::clone(&spill_mode);
        let worker_stopping = Arc::clone(&stopping);
        let worker_pending_finish = Arc::clone(&pending_finish);
        let worker = thread::Builder::new()
            .name("trail-acp-capture".to_string())
            .spawn(move || {
                capture_worker(
                    rx,
                    &workspace_root,
                    coordinator,
                    worker_health,
                    worker_spill,
                    worker_spill_mode,
                    worker_stopping,
                    worker_pending_finish,
                );
                let _ = done_tx.send(());
            })
            .map_err(Error::Io)?;
        Ok(Self {
            tx: Some(tx),
            health,
            worker: Some(worker),
            spill,
            spill_mode,
            stopping,
            pending_finish,
            done_rx: Mutex::new(done_rx),
        })
    }

    pub(crate) fn append(&self, frame: CapturedFrame) -> Result<()> {
        let Some(tx) = &self.tx else {
            return Err(Error::InvalidInput(
                "ACP capture ingress is shut down".to_string(),
            ));
        };
        if self.spill_mode.load(Ordering::Acquire) {
            self.spill_or_degrade(&frame);
            return Ok(());
        }
        self.health.queued.fetch_add(1, Ordering::Relaxed);
        match tx.try_send(CaptureCommand::Frame(frame)) {
            Ok(()) => Ok(()),
            Err(mpsc::TrySendError::Full(CaptureCommand::Frame(frame)))
            | Err(mpsc::TrySendError::Disconnected(CaptureCommand::Frame(frame))) => {
                self.health.queued.fetch_sub(1, Ordering::Relaxed);
                self.spill_mode.store(true, Ordering::Release);
                self.spill_or_degrade(&frame);
                Ok(())
            }
            Err(_) => {
                self.health.queued.fetch_sub(1, Ordering::Relaxed);
                Ok(())
            }
        }
    }

    fn spill_or_degrade(&self, frame: &CapturedFrame) {
        match self.spill.append(frame) {
            Ok(()) => {
                self.health.spilled.fetch_add(1, Ordering::Relaxed);
            }
            Err(error) => {
                if self.health.record_error(&error) {
                    eprintln!("trail acp capture warning: durable spill failed: {error}");
                }
            }
        }
    }

    pub(crate) fn finish(&self, reason: RelayFinishReason) {
        if let Some(tx) = &self.tx {
            if let Err(error) = tx.try_send(CaptureCommand::Finish(reason)) {
                let reason = match error {
                    mpsc::TrySendError::Full(CaptureCommand::Finish(reason))
                    | mpsc::TrySendError::Disconnected(CaptureCommand::Finish(reason)) => reason,
                    _ => return,
                };
                if let Ok(mut pending) = self.pending_finish.lock() {
                    *pending = Some(reason);
                }
            }
        }
    }

    pub(crate) fn flush(&self, timeout: Duration) {
        let Some(tx) = &self.tx else {
            return;
        };
        let deadline = Instant::now() + timeout;
        let (barrier_tx, barrier_rx) = mpsc::channel();
        loop {
            match tx.try_send(CaptureCommand::Barrier(barrier_tx.clone())) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(_)) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(_) => return,
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        let _ = barrier_rx.recv_timeout(remaining);
    }

    #[allow(dead_code)]
    pub(crate) fn shutdown(mut self, timeout: Duration) -> CaptureShutdownReport {
        self.stopping.store(true, Ordering::Release);
        self.tx.take();
        if self
            .done_rx
            .get_mut()
            .is_ok_and(|done_rx| done_rx.recv_timeout(timeout).is_ok())
        {
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
        self.report()
    }

    #[allow(dead_code)]
    fn report(&self) -> CaptureShutdownReport {
        CaptureShutdownReport {
            healthy: self.health.healthy.load(Ordering::Acquire),
            degraded: self.health.degraded.load(Ordering::Acquire),
            queued: self.health.queued.load(Ordering::Acquire),
            spilled: self.health.spilled.load(Ordering::Acquire),
            last_projected_sequence: self.health.last_projected_sequence.load(Ordering::Acquire),
        }
    }
}

impl Drop for CaptureIngress {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        self.tx.take();
        if self
            .done_rx
            .get_mut()
            .is_ok_and(|done_rx| done_rx.recv_timeout(CAPTURE_SHUTDOWN_TIMEOUT).is_ok())
        {
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }
}

struct SpillStore {
    dir: PathBuf,
    state: Mutex<SpillState>,
}

#[derive(Default)]
struct SpillState {
    claimed_paths: Vec<PathBuf>,
}

impl SpillStore {
    fn new(dir: PathBuf) -> Result<Self> {
        fs::create_dir_all(&dir)?;
        Ok(Self {
            dir,
            state: Mutex::new(SpillState::default()),
        })
    }

    fn append(&self, frame: &CapturedFrame) -> Result<()> {
        let _guard = self
            .state
            .lock()
            .map_err(|_| Error::InvalidInput("ACP spill lock poisoned".to_string()))?;
        self.append_locked(std::slice::from_ref(frame))
    }

    fn append_many(&self, frames: &[CapturedFrame]) -> Result<()> {
        let _guard = self
            .state
            .lock()
            .map_err(|_| Error::InvalidInput("ACP spill lock poisoned".to_string()))?;
        self.append_locked(frames)
    }

    fn append_locked(&self, frames: &[CapturedFrame]) -> Result<()> {
        let mut grouped = BTreeMap::<PathBuf, Vec<&CapturedFrame>>::new();
        for frame in frames {
            grouped
                .entry(self.path_for(&frame.connection_id))
                .or_default()
                .push(frame);
        }
        for (path, frames) in grouped {
            let mut file = OpenOptions::new().create(true).append(true).open(path)?;
            for frame in frames {
                serde_json::to_writer(&mut file, frame)?;
                file.write_all(b"\n")?;
            }
            file.sync_data()?;
        }
        Ok(())
    }

    fn take_all(&self) -> Result<Vec<CapturedFrame>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::InvalidInput("ACP spill lock poisoned".to_string()))?;
        if !state.claimed_paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut paths = fs::read_dir(&self.dir)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "jsonl" || extension == "processing")
            })
            .collect::<Vec<_>>();
        paths.sort();
        let mut claimed = Vec::new();
        for path in paths {
            if path
                .extension()
                .is_some_and(|extension| extension == "jsonl")
            {
                let counter = SPILL_CLAIM_COUNTER.fetch_add(1, Ordering::Relaxed);
                let claimed_path = path.with_extension(format!(
                    "jsonl.{}.{}.processing",
                    std::process::id(),
                    counter
                ));
                fs::rename(&path, &claimed_path)?;
                claimed.push(claimed_path);
            } else {
                claimed.push(path);
            }
        }
        let mut frames = Vec::new();
        for path in &claimed {
            let file = OpenOptions::new().read(true).open(path)?;
            for line in BufReader::new(file).lines() {
                let line = line?;
                if !line.trim().is_empty() {
                    frames.push(serde_json::from_str(&line)?);
                }
            }
        }
        state.claimed_paths = claimed;
        Ok(frames)
    }

    fn complete_claimed(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::InvalidInput("ACP spill lock poisoned".to_string()))?;
        for path in state.claimed_paths.drain(..) {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::Io(error)),
            }
        }
        Ok(())
    }

    fn release_claimed(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.claimed_paths.clear();
        }
    }

    fn path_for(&self, connection_id: &str) -> PathBuf {
        self.dir.join(format!("{connection_id}.jsonl"))
    }
}

fn capture_worker(
    rx: mpsc::Receiver<CaptureCommand>,
    workspace_root: &Path,
    coordinator: Arc<Mutex<CaptureCoordinator>>,
    health: Arc<CaptureHealth>,
    spill: Arc<SpillStore>,
    spill_mode: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    pending_finish: Arc<Mutex<Option<RelayFinishReason>>>,
) {
    let mut pending = match recovery_frames(workspace_root, &spill) {
        Ok(frames) => frames.into_iter().map(|frame| (frame, false)).collect(),
        Err(error) => {
            if health.record_error(&error) {
                eprintln!("trail acp capture warning: recovery failed: {error}");
            }
            VecDeque::new()
        }
    };

    loop {
        if stopping.load(Ordering::Acquire) {
            for command in rx.try_iter() {
                match command {
                    CaptureCommand::Frame(frame) => pending.push_back((frame, true)),
                    CaptureCommand::Finish(reason) => store_finish(&pending_finish, reason),
                    CaptureCommand::Barrier(barrier) => {
                        let _ = barrier.send(());
                    }
                }
            }
            let drain_deadline = Instant::now() + CAPTURE_SHUTDOWN_DRAIN_BUDGET;
            while Instant::now() + CAPTURE_PROJECT_LOCK_WAIT < drain_deadline {
                let Some((frame, queued)) = pending.pop_front() else {
                    break;
                };
                if queued {
                    health.queued.fetch_sub(1, Ordering::Relaxed);
                }
                match process_frame(workspace_root, &coordinator, &frame) {
                    Ok(()) => {
                        health.healthy.store(true, Ordering::Release);
                        health
                            .last_projected_sequence
                            .store(frame.sequence, Ordering::Release);
                    }
                    Err(error) => {
                        pending.push_front((frame, false));
                        if health.record_error(&error) {
                            eprintln!("trail acp capture warning: {error}");
                        }
                        break;
                    }
                }
            }
            let frames = pending
                .drain(..)
                .map(|(frame, queued)| {
                    if queued {
                        health.queued.fetch_sub(1, Ordering::Relaxed);
                    }
                    frame
                })
                .collect::<Vec<_>>();
            if let Err(error) = spill.append_many(&frames) {
                if health.record_error(&error) {
                    eprintln!("trail acp capture warning: shutdown spill failed: {error}");
                }
            } else if frames.is_empty() {
                let _ = spill.complete_claimed();
            }
            if let Some(reason) = take_finish(&pending_finish) {
                let _ = process_finish(&coordinator, reason);
            }
            break;
        }

        if let Some((frame, queued)) = pending.pop_front() {
            if queued {
                health.queued.fetch_sub(1, Ordering::Relaxed);
            }
            match process_frame(workspace_root, &coordinator, &frame) {
                Ok(()) => {
                    health.healthy.store(true, Ordering::Release);
                    health
                        .last_projected_sequence
                        .store(frame.sequence, Ordering::Release);
                    if pending.is_empty() {
                        if let Err(error) = spill.complete_claimed() {
                            if health.record_error(&error) {
                                eprintln!(
                                    "trail acp capture warning: spill acknowledgement failed: {error}"
                                );
                            }
                        } else {
                            spill_mode.store(false, Ordering::Release);
                        }
                    }
                }
                Err(error) => {
                    let mut retry = vec![frame];
                    retry.extend(pending.drain(..).map(|(frame, queued)| {
                        if queued {
                            health.queued.fetch_sub(1, Ordering::Relaxed);
                        }
                        frame
                    }));
                    if let Err(spill_error) = spill.append_many(&retry) {
                        if health.record_error(&spill_error) {
                            eprintln!(
                                "trail acp capture warning: {error}; failed to preserve spill: {spill_error}"
                            );
                        }
                    } else if health.record_error(&error) {
                        eprintln!("trail acp capture warning: {error}");
                    }
                    spill.release_claimed();
                    spill_mode.store(true, Ordering::Release);
                }
            }
            continue;
        }

        if let Some(reason) = take_finish(&pending_finish) {
            if let Err(error) = process_finish(&coordinator, reason) {
                health.record_error(&error);
            }
        }

        match rx.recv_timeout(CAPTURE_RETRY_INTERVAL) {
            Ok(CaptureCommand::Frame(frame)) => {
                match spill.take_all() {
                    Ok(frames) => {
                        pending.extend(frames.into_iter().map(|frame| (frame, false)));
                    }
                    Err(error) => {
                        if health.record_error(&error) {
                            eprintln!("trail acp capture warning: spill replay failed: {error}");
                        }
                    }
                }
                pending.push_back((frame, true));
            }
            Ok(CaptureCommand::Finish(reason)) => store_finish(&pending_finish, reason),
            Ok(CaptureCommand::Barrier(barrier)) => {
                if let Some(reason) = take_finish(&pending_finish) {
                    if let Err(error) = process_finish(&coordinator, reason) {
                        health.record_error(&error);
                    }
                }
                let _ = barrier.send(());
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                stopping.store(true, Ordering::Release);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => match spill.take_all() {
                Ok(frames) => {
                    if frames.is_empty() {
                        spill_mode.store(false, Ordering::Release);
                    } else {
                        pending.extend(frames.into_iter().map(|frame| (frame, false)));
                    }
                }
                Err(error) => {
                    if health.record_error(&error) {
                        eprintln!("trail acp capture warning: spill replay failed: {error}");
                    }
                }
            },
        }
    }
}

fn process_frame(
    workspace_root: &Path,
    coordinator: &Arc<Mutex<CaptureCoordinator>>,
    frame: &CapturedFrame,
) -> Result<()> {
    Trail::with_write_lock_wait(CAPTURE_PROJECT_LOCK_WAIT, || {
        let mut db = Trail::open(workspace_root)?;
        let direction = direction_name(frame.direction);
        let report = db.persist_agent_hook_receipt(AgentHookReceiptInput {
            installation_id: None,
            provider: "trail-acp".to_string(),
            native_event: "acp/frame".to_string(),
            native_session_id: session_id(&frame.redacted_message),
            native_turn_id: None,
            transport: AgentCaptureTransport::Acp,
            connection_id: Some(frame.connection_id.clone()),
            direction: Some(direction.to_string()),
            connection_sequence: Some(frame.sequence),
            dedupe_key: format!(
                "acp:{}:{}:{}",
                frame.connection_id, direction, frame.sequence
            ),
            payload: serde_json::json!({
                "connection_id": frame.connection_id,
                "direction": direction,
                "sequence": frame.sequence,
                "received_at": frame.received_at,
                "message": frame.redacted_message.clone(),
                "project": frame.project
            }),
            occurred_at: Some(frame.received_at),
        })?;
        if frame.project && report.receipt.status != "processed" {
            let mut message = frame.redacted_message.clone();
            let mut capture = coordinator.lock().map_err(|_| {
                Error::InvalidInput("ACP capture coordinator lock poisoned".to_string())
            })?;
            match frame.direction {
                Direction::ClientToAgent => capture.before_client_message(&mut message)?,
                Direction::AgentToClient => capture.before_agent_message(&mut message)?,
            }
        }
        db.mark_agent_hook_receipt_processed(&report.receipt.receipt_id)?;
        Ok(())
    })
}

fn recovery_frames(workspace_root: &Path, spill: &SpillStore) -> Result<Vec<CapturedFrame>> {
    let mut frames = spill.take_all()?;
    let db = Trail::open(workspace_root)?;
    for payload in db.pending_acp_capture_payloads()? {
        frames.push(frame_from_receipt_payload(payload)?);
    }
    let mut seen = HashSet::new();
    frames.retain(|frame| {
        seen.insert((frame.connection_id.clone(), frame.direction, frame.sequence))
    });
    frames.sort_by(|left, right| {
        if left.connection_id == right.connection_id {
            left.sequence
                .cmp(&right.sequence)
                .then_with(|| direction_name(left.direction).cmp(direction_name(right.direction)))
        } else {
            left.received_at
                .cmp(&right.received_at)
                .then_with(|| left.connection_id.cmp(&right.connection_id))
        }
    });
    Ok(frames)
}

fn frame_from_receipt_payload(payload: Value) -> Result<CapturedFrame> {
    let direction = match payload.get("direction").and_then(Value::as_str) {
        Some("client_to_agent") => Direction::ClientToAgent,
        Some("agent_to_client") => Direction::AgentToClient,
        other => {
            return Err(Error::Corrupt(format!(
                "ACP receipt has invalid direction {other:?}"
            )));
        }
    };
    Ok(CapturedFrame {
        connection_id: payload
            .get("connection_id")
            .and_then(Value::as_str)
            .ok_or_else(|| Error::Corrupt("ACP receipt is missing connection_id".to_string()))?
            .to_string(),
        direction,
        sequence: payload
            .get("sequence")
            .and_then(Value::as_u64)
            .ok_or_else(|| Error::Corrupt("ACP receipt is missing sequence".to_string()))?,
        received_at: payload
            .get("received_at")
            .and_then(Value::as_i64)
            .ok_or_else(|| Error::Corrupt("ACP receipt is missing received_at".to_string()))?,
        redacted_message: payload
            .get("message")
            .cloned()
            .ok_or_else(|| Error::Corrupt("ACP receipt is missing message".to_string()))?,
        project: payload
            .get("project")
            .and_then(Value::as_bool)
            .unwrap_or(true),
    })
}

fn store_finish(target: &Mutex<Option<RelayFinishReason>>, reason: RelayFinishReason) {
    if let Ok(mut pending) = target.lock() {
        *pending = Some(reason);
    }
}

fn take_finish(target: &Mutex<Option<RelayFinishReason>>) -> Option<RelayFinishReason> {
    target.lock().ok().and_then(|mut pending| pending.take())
}

fn process_finish(
    coordinator: &Arc<Mutex<CaptureCoordinator>>,
    reason: RelayFinishReason,
) -> Result<()> {
    let (status, summary) = match reason {
        RelayFinishReason::EditorEof => ("cancelled", "editor input closed"),
        RelayFinishReason::EditorError(_) => ("failed", "editor sent malformed JSON"),
        RelayFinishReason::AgentEof => ("failed", "upstream output closed"),
        RelayFinishReason::AgentError(_) => ("failed", "upstream sent malformed JSON"),
    };
    let mut capture = coordinator
        .lock()
        .map_err(|_| Error::InvalidInput("ACP capture coordinator lock poisoned".to_string()))?;
    Trail::with_write_lock_wait(CAPTURE_PROJECT_LOCK_WAIT, || {
        capture.finish_open_turns(status, summary)
    })
}

pub(crate) fn capture_frame(
    connection_id: &str,
    direction: Direction,
    sequence: u64,
    message: &Value,
    project: bool,
) -> CapturedFrame {
    CapturedFrame {
        connection_id: connection_id.to_string(),
        direction,
        sequence,
        received_at: now_millis(),
        redacted_message: bounded_redacted_message(message),
        project,
    }
}

fn bounded_redacted_message(message: &Value) -> Value {
    let mut redacted = super::redact_json(message.clone());
    redact_callback_secrets(&mut redacted);
    let bytes = serde_json::to_vec(&redacted).unwrap_or_default();
    if bytes.len() <= CAPTURE_MESSAGE_LIMIT {
        return redacted;
    }
    serde_json::json!({
        "jsonrpc": redacted.get("jsonrpc"),
        "id": redacted.get("id"),
        "method": redacted.get("method"),
        "truncated": true,
        "bytes": bytes.len(),
        "sha256": hex::encode(Sha256::digest(bytes))
    })
}

fn redact_callback_secrets(message: &mut Value) {
    match message.get("method").and_then(Value::as_str) {
        Some("fs/write_text_file") => {
            let Some(content) = message
                .pointer("/params/content")
                .and_then(Value::as_str)
                .map(str::to_string)
            else {
                return;
            };
            let redacted = crate::db::redact_sensitive_text(&content);
            if let Some(params) = message.get_mut("params").and_then(Value::as_object_mut) {
                params.insert(
                    "content".to_string(),
                    serde_json::json!({
                        "redacted": true,
                        "byte_len": content.len(),
                        "sha256": hex::encode(Sha256::digest(redacted.as_bytes()))
                    }),
                );
            }
        }
        Some("terminal/create") => {
            let Some(env) = message
                .pointer_mut("/params/env")
                .and_then(Value::as_array_mut)
            else {
                return;
            };
            for variable in env {
                if let Some(variable) = variable.as_object_mut() {
                    if variable.contains_key("value") {
                        variable
                            .insert("value".to_string(), Value::String("[REDACTED]".to_string()));
                    }
                }
            }
        }
        _ => {}
    }
}

fn direction_name(direction: Direction) -> &'static str {
    match direction {
        Direction::ClientToAgent => "client_to_agent",
        Direction::AgentToClient => "agent_to_client",
    }
}

fn session_id(message: &Value) -> Option<String> {
    message
        .pointer("/params/sessionId")
        .or_else(|| message.pointer("/result/sessionId"))
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| i64::try_from(duration.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::AcpRelayOptions;
    use crate::InitImportMode;

    #[test]
    fn callback_receipts_replace_file_content_and_terminal_environment_values() {
        let write = bounded_redacted_message(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "fs/write_text_file",
            "params": {
                "sessionId": "s",
                "path": "/repo/a.txt",
                "content": "api_key=super-secret"
            }
        }));
        assert_eq!(write["params"]["content"]["redacted"], true);
        assert!(!write.to_string().contains("super-secret"));

        let terminal = bounded_redacted_message(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": "terminal",
            "method": "terminal/create",
            "params": {
                "sessionId": "s",
                "command": "echo",
                "env": [{"name": "API_TOKEN", "value": "opaque-secret"}]
            }
        }));
        assert_eq!(terminal["params"]["env"][0]["value"], "[REDACTED]");
        assert!(!terminal.to_string().contains("opaque-secret"));
    }

    #[test]
    fn queue_overflow_spills_every_frame_and_shutdown_is_bounded() {
        const FRAME_COUNT: u64 = CAPTURE_QUEUE_CAPACITY as u64 + 2;

        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "overflow fixture\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let options = AcpRelayOptions {
            workspace_root: temp.path().to_path_buf(),
            db_dir: temp.path().join(".trail"),
            lane: None,
            from_ref: None,
            provider: Some("fixture".to_string()),
            model: None,
            materialize: false,
            workdir: None,
            inject_mcp: false,
            upstream_command: vec!["fixture".to_string()],
            upstream_env: BTreeMap::new(),
        };
        let coordinator = Arc::new(Mutex::new(CaptureCoordinator::new(options).unwrap()));
        fs::write(
            temp.path().join(".trail/lock"),
            format!("pid={} created_at=0\n", std::process::id()),
        )
        .unwrap();
        let ingress = CaptureIngress::new(
            temp.path().to_path_buf(),
            temp.path().join(".trail"),
            coordinator,
            "overflow-connection".to_string(),
        )
        .unwrap();
        for sequence in 0..FRAME_COUNT {
            ingress
                .append(capture_frame(
                    "overflow-connection",
                    Direction::AgentToClient,
                    sequence,
                    &serde_json::json!({
                        "jsonrpc": "2.0",
                        "method": "session/update",
                        "params": {"sequence": sequence}
                    }),
                    false,
                ))
                .unwrap();
        }
        let shutdown_started = Instant::now();
        let report = ingress.shutdown(CAPTURE_SHUTDOWN_TIMEOUT);
        assert!(shutdown_started.elapsed() < CAPTURE_SHUTDOWN_TIMEOUT + Duration::from_millis(250));
        assert!(
            report.spilled >= 1,
            "the bounded queue never entered spill mode"
        );
        fs::remove_file(temp.path().join(".trail/lock")).unwrap();

        let preserved = fs::read_dir(temp.path().join(".trail/acp-ingress"))
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| fs::read_to_string(entry.path()).unwrap())
            .flat_map(|contents| {
                contents
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter_map(|line| serde_json::from_str::<CapturedFrame>(&line).ok())
            .map(|frame| frame.sequence)
            .collect::<HashSet<_>>();
        assert_eq!(preserved.len(), usize::try_from(FRAME_COUNT).unwrap());
    }
}
