#[cfg(test)]
use std::collections::HashMap;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs::{self, File, OpenOptions};
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

pub(crate) const ACP_CAPTURE_QUEUE_CAPACITY: usize = 4_096;
const CAPTURE_MESSAGE_LIMIT: usize = 512 * 1024;
const CAPTURE_PROJECT_LOCK_WAIT: Duration = Duration::from_millis(250);
const CAPTURE_RETRY_INTERVAL: Duration = Duration::from_millis(25);
const CAPTURE_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);
const CAPTURE_SHUTDOWN_DRAIN_BUDGET: Duration = Duration::from_millis(500);
// Test-only liveness cap for observing detached cleanup after the unchanged shutdown deadline.
#[cfg(test)]
const CAPTURE_TEST_WORKER_COMPLETION_LIVENESS_CAP: Duration = Duration::from_secs(10);
static SPILL_CLAIM_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
static CAPTURE_WORKER_PAUSES: std::sync::OnceLock<Mutex<HashMap<PathBuf, Arc<AtomicBool>>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
static CAPTURE_PROJECTION_ATTEMPTS: std::sync::OnceLock<Mutex<HashMap<PathBuf, Arc<AtomicUsize>>>> =
    std::sync::OnceLock::new();

#[cfg(test)]
fn pause_capture_worker_for_test(
    workspace_root: &Path,
) -> (
    crate::test_support::scoped_state::ScopedTestState<PathBuf, Arc<AtomicBool>>,
    Arc<AtomicBool>,
) {
    let key = fs::canonicalize(workspace_root).unwrap();
    let paused = Arc::new(AtomicBool::new(true));
    let pauses = CAPTURE_WORKER_PAUSES.get_or_init(|| Mutex::new(HashMap::new()));
    let guard = crate::test_support::scoped_state::ScopedTestState::install(
        pauses,
        key,
        Arc::clone(&paused),
    );
    (guard, paused)
}

#[cfg(test)]
fn count_capture_projection_attempts_for_test(
    workspace_root: &Path,
) -> (
    crate::test_support::scoped_state::ScopedTestState<PathBuf, Arc<AtomicUsize>>,
    Arc<AtomicUsize>,
) {
    let key = fs::canonicalize(workspace_root).unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let counters = CAPTURE_PROJECTION_ATTEMPTS.get_or_init(|| Mutex::new(HashMap::new()));
    let guard = crate::test_support::scoped_state::ScopedTestState::install(
        counters,
        key,
        Arc::clone(&attempts),
    );
    (guard, attempts)
}

#[cfg(test)]
fn record_capture_projection_attempt_for_test(coordinator: &Arc<Mutex<CaptureCoordinator>>) {
    let workspace_root = coordinator
        .lock()
        .ok()
        .map(|coordinator| coordinator.options.workspace_root.clone());
    let Some(workspace_root) = workspace_root else {
        return;
    };
    let key = fs::canonicalize(&workspace_root).unwrap_or(workspace_root);
    if let Some(attempts) = CAPTURE_PROJECTION_ATTEMPTS
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .get(&key)
    {
        attempts.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
fn wait_for_capture_worker_test_pause(workspace_root: &Path) {
    let key = fs::canonicalize(workspace_root).unwrap_or_else(|_| workspace_root.to_path_buf());
    loop {
        let paused = CAPTURE_WORKER_PAUSES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&key)
            .is_some_and(|paused| paused.load(Ordering::Acquire));
        if !paused {
            return;
        }
        thread::yield_now();
    }
}

#[cfg(test)]
fn wait_for_capture_test_signal(signal: &AtomicBool, timeout: Duration, message: &str) {
    let deadline = Instant::now() + timeout;
    while !signal.load(Ordering::Acquire) {
        assert!(Instant::now() < deadline, "{message}");
        thread::sleep(Duration::from_millis(1));
    }
}

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
    #[cfg(test)]
    SimulateWorkerPanic,
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
    #[cfg(test)]
    worker_completed: Arc<AtomicBool>,
    pending_finish: Arc<Mutex<Option<RelayFinishReason>>>,
    done_rx: Mutex<mpsc::Receiver<()>>,
}

impl CaptureIngress {
    pub(crate) fn new(
        _workspace_root: PathBuf,
        db_dir: PathBuf,
        coordinator: Arc<Mutex<CaptureCoordinator>>,
        connection_id: String,
    ) -> Result<Self> {
        let spill = Arc::new(SpillStore::new(db_dir.join("acp-ingress"), connection_id)?);
        let health = Arc::new(CaptureHealth::new());
        let spill_mode = Arc::new(AtomicBool::new(false));
        let stopping = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let worker_completed = Arc::new(AtomicBool::new(false));
        let pending_finish = Arc::new(Mutex::new(None));
        let (tx, rx) = mpsc::sync_channel(ACP_CAPTURE_QUEUE_CAPACITY);
        let (done_tx, done_rx) = mpsc::channel();
        let worker_health = Arc::clone(&health);
        let worker_spill = Arc::clone(&spill);
        let worker_spill_mode = Arc::clone(&spill_mode);
        let worker_stopping = Arc::clone(&stopping);
        #[cfg(test)]
        let worker_completion_signal = Arc::clone(&worker_completed);
        let worker_pending_finish = Arc::clone(&pending_finish);
        let worker = thread::Builder::new()
            .name("trail-acp-capture".to_string())
            .spawn(move || {
                #[cfg(test)]
                wait_for_capture_worker_test_pause(&_workspace_root);
                #[cfg(test)]
                drop(_workspace_root);
                let panic_health = Arc::clone(&worker_health);
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    capture_worker(
                        rx,
                        coordinator,
                        worker_health,
                        worker_spill,
                        worker_spill_mode,
                        worker_stopping,
                        worker_pending_finish,
                    );
                }));
                if outcome.is_err() {
                    panic_health.record_error(&Error::InvalidInput(
                        "ACP capture worker panicked".to_string(),
                    ));
                }
                drop(outcome);
                drop(panic_health);
                #[cfg(test)]
                worker_completion_signal.store(true, Ordering::Release);
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
            #[cfg(test)]
            worker_completed,
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
        if let Err(error) = self.spill.persist_finish(&reason)
            && self.health.record_error(&error)
        {
            eprintln!("trail acp capture warning: durable finish journal failed: {error}");
        }
        if let Some(tx) = &self.tx
            && let Err(error) = tx.try_send(CaptureCommand::Finish(reason))
        {
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

    pub(crate) fn flush(&self, timeout: Duration) -> bool {
        let Some(tx) = &self.tx else {
            return false;
        };
        let deadline = Instant::now() + timeout;
        let (barrier_tx, barrier_rx) = mpsc::channel();
        loop {
            match tx.try_send(CaptureCommand::Barrier(barrier_tx.clone())) {
                Ok(()) => break,
                Err(mpsc::TrySendError::Full(_)) if Instant::now() < deadline => {
                    thread::sleep(Duration::from_millis(1));
                }
                Err(_) => return false,
            }
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        barrier_rx.recv_timeout(remaining).is_ok()
    }

    #[allow(dead_code)]
    pub(crate) fn shutdown(mut self, timeout: Duration) -> CaptureShutdownReport {
        let deadline = Instant::now() + timeout;
        self.stopping.store(true, Ordering::Release);
        self.tx.take();
        let completed = self.wait_for_worker(deadline.saturating_duration_since(Instant::now()));
        if !completed {
            self.worker.take();
            self.health.record_error(&Error::InvalidInput(format!(
                "ACP capture shutdown timed out after {}ms",
                timeout.as_millis()
            )));
        }
        let mut report = self.report();
        if !completed {
            report.healthy = false;
            report.degraded = true;
        }
        report
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

    fn wait_for_worker(&mut self, timeout: Duration) -> bool {
        if self.worker.is_none() {
            return true;
        }
        let completed = self
            .done_rx
            .get_mut()
            .is_ok_and(|done_rx| done_rx.recv_timeout(timeout).is_ok());
        if completed {
            drop(self.worker.take());
        }
        completed
    }
}

impl Drop for CaptureIngress {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::Release);
        self.tx.take();
        self.wait_for_worker(CAPTURE_SHUTDOWN_TIMEOUT);
    }
}

struct SpillStore {
    dir: PathBuf,
    connection_id: String,
    owner_path: PathBuf,
    _owner_lock: File,
    state: Mutex<SpillState>,
}

#[derive(Default)]
struct SpillState {
    claimed_paths: Vec<PathBuf>,
    claimed_finish_paths: Vec<PathBuf>,
    recovered_finishes: VecDeque<RelayFinishReason>,
    claimed_owners: Vec<(PathBuf, File)>,
}

impl SpillStore {
    fn new(dir: PathBuf, connection_id: String) -> Result<Self> {
        fs::create_dir_all(&dir)?;
        let owner_path = dir.join(format!("{connection_id}.owner"));
        let owner_lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&owner_path)?;
        lock_spill_owner(&owner_lock)?;
        Ok(Self {
            dir,
            connection_id,
            owner_path,
            _owner_lock: owner_lock,
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

    fn persist_finish(&self, reason: &RelayFinishReason) -> Result<()> {
        let _guard = self
            .state
            .lock()
            .map_err(|_| Error::InvalidInput("ACP spill lock poisoned".to_string()))?;
        let counter = SPILL_CLAIM_COUNTER.fetch_add(1, Ordering::Relaxed);
        let temporary = self.dir.join(format!(
            "{}.finish.{}.{}.tmp",
            self.connection_id,
            std::process::id(),
            counter
        ));
        let final_path = self.finish_path_for(&self.connection_id);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)?;
        serde_json::to_writer(&mut file, reason)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, &final_path)?;
        File::open(&self.dir)?.sync_all()?;
        Ok(())
    }

    fn take_all(&self) -> Result<Vec<CapturedFrame>> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::InvalidInput("ACP spill lock poisoned".to_string()))?;
        if !state.claimed_paths.is_empty() || !state.claimed_finish_paths.is_empty() {
            return Ok(Vec::new());
        }
        let mut paths = fs::read_dir(&self.dir)?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                spill_connection_id(path).is_some()
                    && path.extension().is_some_and(|extension| {
                        extension == "jsonl" || extension == "json" || extension == "processing"
                    })
            })
            .collect::<Vec<_>>();
        paths.sort();
        let mut claimed = Vec::new();
        let mut claimed_finishes = Vec::new();
        let mut recoverable_connections = HashSet::new();
        recoverable_connections.insert(self.connection_id.clone());
        let mut active_connections = HashSet::new();
        let mut claimed_owners = Vec::new();
        for path in paths {
            let Some(connection_id) = spill_connection_id(&path) else {
                continue;
            };
            if !recoverable_connections.contains(&connection_id)
                && !active_connections.contains(&connection_id)
            {
                let owner_path = self.dir.join(format!("{connection_id}.owner"));
                let owner = OpenOptions::new()
                    .read(true)
                    .write(true)
                    .create(true)
                    .truncate(false)
                    .open(&owner_path)?;
                if try_lock_spill_owner(&owner)? {
                    recoverable_connections.insert(connection_id.clone());
                    claimed_owners.push((owner_path, owner));
                } else {
                    active_connections.insert(connection_id.clone());
                }
            }
            if active_connections.contains(&connection_id) {
                continue;
            }
            let finish = spill_finish_path(&path);
            if !path
                .extension()
                .is_some_and(|extension| extension == "processing")
            {
                let counter = SPILL_CLAIM_COUNTER.fetch_add(1, Ordering::Relaxed);
                let claimed_path = path.with_extension(format!(
                    "{}.{}.{}.processing",
                    path.extension()
                        .and_then(|value| value.to_str())
                        .unwrap_or("spill"),
                    std::process::id(),
                    counter
                ));
                fs::rename(&path, &claimed_path)?;
                if finish {
                    claimed_finishes.push(claimed_path);
                } else {
                    claimed.push(claimed_path);
                }
            } else if finish {
                claimed_finishes.push(path);
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
        let mut finishes = VecDeque::new();
        for path in &claimed_finishes {
            let file = OpenOptions::new().read(true).open(path)?;
            finishes.push_back(serde_json::from_reader(file)?);
        }
        state.claimed_paths = claimed;
        state.claimed_finish_paths = claimed_finishes;
        state.recovered_finishes = finishes;
        state.claimed_owners = claimed_owners;
        Ok(frames)
    }

    fn take_recovered_finishes(&self) -> Vec<RelayFinishReason> {
        self.state
            .lock()
            .map(|mut state| state.recovered_finishes.drain(..).collect())
            .unwrap_or_default()
    }

    fn complete_claimed_frames(&self) -> Result<()> {
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
        if state.claimed_finish_paths.is_empty() {
            remove_claimed_owners(&mut state)?;
        }
        Ok(())
    }

    fn complete_finish(&self) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::InvalidInput("ACP spill lock poisoned".to_string()))?;
        for path in state.claimed_finish_paths.drain(..) {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(Error::Io(error)),
            }
        }
        match fs::remove_file(self.finish_path_for(&self.connection_id)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::Io(error)),
        }
        state.recovered_finishes.clear();
        remove_claimed_owners(&mut state)?;
        File::open(&self.dir)?.sync_all()?;
        Ok(())
    }

    fn finish_path_for(&self, connection_id: &str) -> PathBuf {
        self.dir.join(format!("{connection_id}.finish.json"))
    }

    fn release_claimed(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.claimed_paths.clear();
            state.claimed_finish_paths.clear();
            state.recovered_finishes.clear();
            state.claimed_owners.clear();
        }
    }

    fn path_for(&self, connection_id: &str) -> PathBuf {
        self.dir.join(format!("{connection_id}.jsonl"))
    }
}

fn remove_claimed_owners(state: &mut SpillState) -> Result<()> {
    for (owner_path, _owner) in state.claimed_owners.drain(..) {
        match fs::remove_file(owner_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(Error::Io(error)),
        }
    }
    Ok(())
}

impl Drop for SpillStore {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.owner_path);
    }
}

fn spill_connection_id(path: &Path) -> Option<String> {
    let name = path.file_name()?.to_str()?;
    let connection_id = name
        .split_once(".jsonl")
        .or_else(|| name.split_once(".finish.json"))?
        .0;
    (!connection_id.is_empty()).then(|| connection_id.to_string())
}

fn spill_finish_path(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains(".finish.json"))
}

#[cfg(unix)]
fn lock_spill_owner(file: &File) -> Result<()> {
    match rustix::fs::flock(file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(()),
        Err(error) if error == rustix::io::Errno::WOULDBLOCK => Err(Error::InvalidInput(
            "ACP capture connection identity is already active".to_string(),
        )),
        Err(error) => Err(Error::Io(error.into())),
    }
}

#[cfg(not(unix))]
fn lock_spill_owner(_file: &File) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn try_lock_spill_owner(file: &File) -> Result<bool> {
    match rustix::fs::flock(file, rustix::fs::FlockOperation::NonBlockingLockExclusive) {
        Ok(()) => Ok(true),
        Err(error) if error == rustix::io::Errno::WOULDBLOCK => Ok(false),
        Err(error) => Err(Error::Io(error.into())),
    }
}

#[cfg(not(unix))]
fn try_lock_spill_owner(_file: &File) -> Result<bool> {
    Ok(true)
}

#[allow(clippy::too_many_arguments)]
fn capture_worker(
    rx: mpsc::Receiver<CaptureCommand>,
    coordinator: Arc<Mutex<CaptureCoordinator>>,
    health: Arc<CaptureHealth>,
    spill: Arc<SpillStore>,
    spill_mode: Arc<AtomicBool>,
    stopping: Arc<AtomicBool>,
    pending_finish: Arc<Mutex<Option<RelayFinishReason>>>,
) {
    let mut pending = match recovery_frames(&coordinator, &spill) {
        Ok(frames) => frames.into_iter().map(|frame| (frame, false)).collect(),
        Err(error) => {
            if health.record_error(&error) {
                eprintln!("trail acp capture warning: recovery failed: {error}");
            }
            VecDeque::new()
        }
    };
    load_recovered_finish(&spill, &pending_finish);
    let mut deferred_barriers = Vec::new();

    loop {
        if stopping.load(Ordering::Acquire) {
            let mut preserve_for_replay = spill_mode.load(Ordering::Acquire);
            let mut recovered_spill = true;
            match spill.take_all() {
                Ok(frames) => {
                    preserve_for_replay |= !frames.is_empty();
                    pending.extend(frames.into_iter().map(|frame| (frame, false)));
                    load_recovered_finish(&spill, &pending_finish);
                }
                Err(error) => {
                    recovered_spill = false;
                    if health.record_error(&error) {
                        eprintln!(
                            "trail acp capture warning: shutdown spill replay failed: {error}"
                        );
                    }
                }
            }
            for command in rx.try_iter() {
                match command {
                    CaptureCommand::Frame(frame) => pending.push_back((frame, true)),
                    CaptureCommand::Finish(reason) => store_finish(&pending_finish, reason),
                    CaptureCommand::Barrier(barrier) => {
                        deferred_barriers.push(barrier);
                    }
                    #[cfg(test)]
                    CaptureCommand::SimulateWorkerPanic => {
                        panic!("simulated ACP capture worker panic")
                    }
                }
            }
            preserve_for_replay |= pending.iter().any(|(_, queued)| !queued);
            preserve_for_replay |= pending_finish
                .lock()
                .is_ok_and(|pending_finish| pending_finish.is_some());
            sort_pending_frames(&mut pending);
            let drain_deadline = Instant::now() + CAPTURE_SHUTDOWN_DRAIN_BUDGET;
            while !preserve_for_replay
                && Instant::now() + CAPTURE_PROJECT_LOCK_WAIT < drain_deadline
            {
                let Some((frame, queued)) = pending.pop_front() else {
                    break;
                };
                if queued {
                    health.queued.fetch_sub(1, Ordering::Relaxed);
                }
                match process_frame(&coordinator, &frame) {
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
            let preserved_every_frame = recovered_spill
                && match spill.append_many(&frames) {
                    Err(error) => {
                        if health.record_error(&error) {
                            eprintln!("trail acp capture warning: shutdown spill failed: {error}");
                        }
                        false
                    }
                    _ => {
                        if frames.is_empty() {
                            let _ = spill.complete_claimed_frames();
                        }
                        frames.is_empty()
                    }
                };
            if !preserve_for_replay
                && preserved_every_frame
                && settle_pending_finish(&coordinator, &health, &spill, &pending_finish)
            {
                acknowledge_barriers(&mut deferred_barriers);
            }
            break;
        }

        let mut merged_frames = false;
        if spill_mode.load(Ordering::Acquire) {
            for command in rx.try_iter() {
                match command {
                    CaptureCommand::Frame(frame) => {
                        pending.push_back((frame, true));
                        merged_frames = true;
                    }
                    CaptureCommand::Finish(reason) => store_finish(&pending_finish, reason),
                    CaptureCommand::Barrier(barrier) => deferred_barriers.push(barrier),
                    #[cfg(test)]
                    CaptureCommand::SimulateWorkerPanic => {
                        panic!("simulated ACP capture worker panic")
                    }
                }
            }
            match spill.take_all() {
                Ok(frames) => {
                    merged_frames |= !frames.is_empty();
                    pending.extend(frames.into_iter().map(|frame| (frame, false)));
                    load_recovered_finish(&spill, &pending_finish);
                }
                Err(error) => {
                    if health.record_error(&error) {
                        eprintln!("trail acp capture warning: spill replay failed: {error}");
                    }
                }
            }
        }
        if merged_frames {
            sort_pending_frames(&mut pending);
        }

        if let Some((frame, queued)) = pending.pop_front() {
            if queued {
                health.queued.fetch_sub(1, Ordering::Relaxed);
            }
            match process_frame(&coordinator, &frame) {
                Ok(()) => {
                    health.healthy.store(true, Ordering::Release);
                    health
                        .last_projected_sequence
                        .store(frame.sequence, Ordering::Release);
                    if pending.is_empty() {
                        match spill.complete_claimed_frames() {
                            Err(error) => {
                                if health.record_error(&error) {
                                    eprintln!(
                                        "trail acp capture warning: spill acknowledgement failed: {error}"
                                    );
                                }
                            }
                            _ => {
                                spill_mode.store(false, Ordering::Release);
                            }
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
                    match spill.append_many(&retry) {
                        Err(spill_error) => {
                            if health.record_error(&spill_error) {
                                eprintln!(
                                    "trail acp capture warning: {error}; failed to preserve spill: {spill_error}"
                                );
                            }
                        }
                        _ => {
                            if health.record_error(&error) {
                                eprintln!("trail acp capture warning: {error}");
                            }
                        }
                    }
                    spill.release_claimed();
                    spill_mode.store(true, Ordering::Release);
                }
            }
            continue;
        }

        if !spill_mode.load(Ordering::Acquire)
            && settle_pending_finish(&coordinator, &health, &spill, &pending_finish)
        {
            acknowledge_barriers(&mut deferred_barriers);
        }

        match rx.recv_timeout(CAPTURE_RETRY_INTERVAL) {
            Ok(CaptureCommand::Frame(frame)) => pending.push_back((frame, true)),
            Ok(CaptureCommand::Finish(reason)) => store_finish(&pending_finish, reason),
            Ok(CaptureCommand::Barrier(barrier)) => {
                deferred_barriers.push(barrier);
            }
            #[cfg(test)]
            Ok(CaptureCommand::SimulateWorkerPanic) => {
                panic!("simulated ACP capture worker panic")
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                stopping.store(true, Ordering::Release);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

fn sort_pending_frames(pending: &mut VecDeque<(CapturedFrame, bool)>) {
    let mut ordered = pending.drain(..).collect::<Vec<_>>();
    ordered.sort_by(|(left, _), (right, _)| {
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
    pending.extend(ordered);
}

fn settle_pending_finish(
    coordinator: &Arc<Mutex<CaptureCoordinator>>,
    health: &CaptureHealth,
    spill: &SpillStore,
    pending_finish: &Mutex<Option<RelayFinishReason>>,
) -> bool {
    let Some(reason) = take_finish(pending_finish) else {
        return true;
    };
    match process_finish(coordinator, reason.clone()).and_then(|()| spill.complete_finish()) {
        Ok(()) => true,
        Err(error) => {
            store_finish(pending_finish, reason);
            health.record_error(&error);
            false
        }
    }
}

fn load_recovered_finish(spill: &SpillStore, pending_finish: &Mutex<Option<RelayFinishReason>>) {
    for reason in spill.take_recovered_finishes() {
        store_finish(pending_finish, reason);
    }
}

fn acknowledge_barriers(barriers: &mut Vec<mpsc::Sender<()>>) {
    for barrier in barriers.drain(..) {
        let _ = barrier.send(());
    }
}

fn process_frame(
    coordinator: &Arc<Mutex<CaptureCoordinator>>,
    frame: &CapturedFrame,
) -> Result<()> {
    #[cfg(test)]
    record_capture_projection_attempt_for_test(coordinator);
    Trail::with_write_lock_wait(CAPTURE_PROJECT_LOCK_WAIT, || {
        let mut db = {
            let capture = coordinator.lock().map_err(|_| {
                Error::InvalidInput("ACP capture coordinator lock poisoned".to_string())
            })?;
            capture.open_db()?
        };
        let direction = direction_name(frame.direction);
        let input = AgentHookReceiptInput {
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
        };
        if !frame.project {
            db.persist_agent_hook_receipt_processed(input)?;
            return Ok(());
        }
        let report = db.persist_agent_hook_receipt(input)?;
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

fn recovery_frames(
    coordinator: &Arc<Mutex<CaptureCoordinator>>,
    spill: &SpillStore,
) -> Result<Vec<CapturedFrame>> {
    let db = {
        let capture = coordinator.lock().map_err(|_| {
            Error::InvalidInput("ACP capture coordinator lock poisoned".to_string())
        })?;
        capture.open_db()?
    };
    let payloads = db.pending_acp_capture_payloads()?;
    let mut frames = spill.take_all()?;
    for payload in payloads {
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
    #[cfg(test)]
    record_capture_projection_attempt_for_test(coordinator);
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
                if let Some(variable) = variable.as_object_mut()
                    && variable.contains_key("value")
                {
                    variable.insert("value".to_string(), Value::String("[REDACTED]".to_string()));
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

    fn preserved_spill_state(dir: &Path) -> (HashSet<u64>, bool) {
        let mut finish_preserved = false;
        let frames = fs::read_dir(dir)
            .unwrap()
            .filter_map(|entry| entry.ok())
            .flat_map(|entry| {
                finish_preserved |= spill_finish_path(&entry.path());
                fs::read_to_string(entry.path())
                    .unwrap()
                    .lines()
                    .filter(|line| !line.trim().is_empty())
                    .map(str::to_string)
                    .collect::<Vec<_>>()
            })
            .filter_map(|line| serde_json::from_str::<CapturedFrame>(&line).ok())
            .map(|frame| frame.sequence)
            .collect();
        (frames, finish_preserved)
    }

    #[test]
    fn queue_overflow_spills_every_frame_and_shutdown_is_bounded() {
        const FRAME_COUNT: u64 = ACP_CAPTURE_QUEUE_CAPACITY as u64 + 2;

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
        let (_projection_guard, projection_attempts) =
            count_capture_projection_attempts_for_test(temp.path());
        let (_pause_guard, paused) = pause_capture_worker_for_test(temp.path());
        let lock = crate::db::acquire_workspace_lock(&temp.path().join(".trail")).unwrap();
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
        ingress.finish(RelayFinishReason::EditorEof);
        let stopping = Arc::clone(&ingress.stopping);
        let worker_completed = Arc::clone(&ingress.worker_completed);
        let shutdown_thread = thread::spawn(move || {
            let shutdown_started = Instant::now();
            let report = ingress.shutdown(CAPTURE_SHUTDOWN_TIMEOUT);
            (shutdown_started.elapsed(), report)
        });
        wait_for_capture_test_signal(
            &stopping,
            CAPTURE_SHUTDOWN_TIMEOUT,
            "shutdown did not publish stopping state",
        );
        paused.store(false, Ordering::Release);
        let (shutdown_elapsed, report) = shutdown_thread.join().unwrap();
        assert!(shutdown_elapsed < CAPTURE_SHUTDOWN_TIMEOUT + Duration::from_millis(250));
        assert!(
            report.spilled >= 1,
            "the bounded queue never entered spill mode"
        );
        drop(lock);
        wait_for_capture_test_signal(
            &worker_completed,
            CAPTURE_TEST_WORKER_COMPLETION_LIVENESS_CAP,
            "capture worker did not complete durable spill cleanup",
        );

        let spill_dir = temp.path().join(".trail/acp-ingress");
        assert_eq!(
            projection_attempts.load(Ordering::Relaxed),
            0,
            "shutdown attempted a known-contended spill projection"
        );
        let (preserved, finish_preserved) = preserved_spill_state(&spill_dir);
        assert!(
            finish_preserved,
            "shutdown lost the durable finish reason needed for replay"
        );
        assert_eq!(preserved.len(), usize::try_from(FRAME_COUNT).unwrap());
    }

    #[test]
    fn explicit_shutdown_timeout_consumes_only_one_wait_budget() {
        const EXPLICIT_TIMEOUT: Duration = Duration::from_millis(50);

        let temp = tempfile::tempdir().unwrap();
        let (command_tx, _command_rx) = mpsc::sync_channel(1);
        let (done_tx, done_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = thread::spawn(move || {
            let _ = release_rx.recv();
            let _ = done_tx.send(());
        });
        let ingress = CaptureIngress {
            tx: Some(command_tx),
            health: Arc::new(CaptureHealth::new()),
            worker: Some(worker),
            spill: Arc::new(
                SpillStore::new(temp.path().to_path_buf(), "blocked-worker".to_string()).unwrap(),
            ),
            spill_mode: Arc::new(AtomicBool::new(false)),
            stopping: Arc::new(AtomicBool::new(false)),
            worker_completed: Arc::new(AtomicBool::new(false)),
            pending_finish: Arc::new(Mutex::new(None)),
            done_rx: Mutex::new(done_rx),
        };
        let (report_tx, report_rx) = mpsc::channel();
        let shutdown_thread = thread::spawn(move || {
            let started = Instant::now();
            let report = ingress.shutdown(EXPLICIT_TIMEOUT);
            let _ = report_tx.send((started.elapsed(), report));
        });

        let result = report_rx.recv_timeout(EXPLICIT_TIMEOUT + Duration::from_millis(250));
        let _ = release_tx.send(());
        shutdown_thread.join().unwrap();
        let (elapsed, report) = result.expect("explicit shutdown waited again during Drop");
        assert!(
            elapsed < EXPLICIT_TIMEOUT + Duration::from_millis(250),
            "explicit shutdown exceeded its single wait budget: {elapsed:?}"
        );
        assert!(!report.healthy, "timed-out shutdown reported healthy");
        assert!(report.degraded, "timed-out shutdown was not degraded");
    }

    #[test]
    fn shutdown_preserves_startup_replay_without_projection() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(temp.path().join("README.md"), "recovery fixture\n").unwrap();
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
        let spill_dir = temp.path().join(".trail/acp-ingress");
        let prior = SpillStore::new(spill_dir.clone(), "prior-connection".to_string()).unwrap();
        prior
            .append(&capture_frame(
                "prior-connection",
                Direction::AgentToClient,
                7,
                &serde_json::json!({"jsonrpc":"2.0","method":"ext/recover"}),
                false,
            ))
            .unwrap();
        drop(prior);

        let (_projection_guard, projection_attempts) =
            count_capture_projection_attempts_for_test(temp.path());
        let (_pause_guard, paused) = pause_capture_worker_for_test(temp.path());
        let lock = crate::db::acquire_workspace_lock(&temp.path().join(".trail")).unwrap();
        let ingress = CaptureIngress::new(
            temp.path().to_path_buf(),
            temp.path().join(".trail"),
            coordinator,
            "recovery-connection".to_string(),
        )
        .unwrap();
        let stopping = Arc::clone(&ingress.stopping);
        let worker_completed = Arc::clone(&ingress.worker_completed);
        let shutdown_thread = thread::spawn(move || ingress.shutdown(CAPTURE_SHUTDOWN_TIMEOUT));
        wait_for_capture_test_signal(
            &stopping,
            CAPTURE_SHUTDOWN_TIMEOUT,
            "shutdown did not publish stopping state",
        );
        paused.store(false, Ordering::Release);
        shutdown_thread.join().unwrap();
        drop(lock);
        wait_for_capture_test_signal(
            &worker_completed,
            CAPTURE_TEST_WORKER_COMPLETION_LIVENESS_CAP,
            "capture worker did not complete durable replay cleanup",
        );

        assert_eq!(
            projection_attempts.load(Ordering::Relaxed),
            0,
            "shutdown projected a frame already held for durable replay"
        );
        let (preserved, _finish_preserved) = preserved_spill_state(&spill_dir);
        assert_eq!(preserved, HashSet::from([7]));
    }

    fn test_ingress(temp: &Path, connection_id: &str) -> CaptureIngress {
        fs::write(temp.join("README.md"), "capture fault fixture\n").unwrap();
        Trail::init(temp, "main", InitImportMode::WorkingTree, false).unwrap();
        let options = AcpRelayOptions {
            workspace_root: temp.to_path_buf(),
            db_dir: temp.join(".trail"),
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
        CaptureIngress::new(
            temp.to_path_buf(),
            temp.join(".trail"),
            Arc::new(Mutex::new(CaptureCoordinator::new(options).unwrap())),
            connection_id.to_string(),
        )
        .unwrap()
    }

    #[test]
    fn worker_panic_degrades_capture_and_subsequent_frames_spill_durably() {
        let temp = tempfile::tempdir().unwrap();
        let ingress = test_ingress(temp.path(), "panic-connection");
        ingress
            .tx
            .as_ref()
            .unwrap()
            .send(CaptureCommand::SimulateWorkerPanic)
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while !ingress.worker.as_ref().unwrap().is_finished() {
            assert!(Instant::now() < deadline, "capture worker did not panic");
            thread::sleep(Duration::from_millis(1));
        }
        ingress
            .append(capture_frame(
                "panic-connection",
                Direction::AgentToClient,
                1,
                &serde_json::json!({"jsonrpc":"2.0","method":"ext/after-panic"}),
                false,
            ))
            .unwrap();
        let report = ingress.shutdown(Duration::from_millis(100));
        assert!(report.degraded);
        assert_eq!(report.spilled, 1);
    }

    #[test]
    fn spill_write_failure_marks_capture_unhealthy_without_blocking_forwarding() {
        let temp = tempfile::tempdir().unwrap();
        let ingress = test_ingress(temp.path(), "spill-failure");
        let spill_dir = temp.path().join(".trail/acp-ingress");
        fs::remove_file(&ingress.spill.owner_path).unwrap();
        fs::remove_dir(&spill_dir).unwrap();
        fs::write(&spill_dir, "not a directory").unwrap();
        ingress.spill_mode.store(true, Ordering::Release);

        ingress
            .append(capture_frame(
                "spill-failure",
                Direction::ClientToAgent,
                1,
                &serde_json::json!({"jsonrpc":"2.0","method":"ext/spill-failure"}),
                false,
            ))
            .unwrap();
        let report = ingress.shutdown(Duration::from_millis(250));
        assert!(!report.healthy);
        assert!(report.degraded);
        assert_eq!(report.spilled, 0);
    }

    #[test]
    fn barrier_waits_for_spill_replay_and_finish_projection() {
        let temp = tempfile::tempdir().unwrap();
        let ingress = test_ingress(temp.path(), "barrier-order");
        let writer_lock = crate::db::acquire_workspace_lock(&temp.path().join(".trail")).unwrap();

        ingress
            .append(capture_frame(
                "barrier-order",
                Direction::AgentToClient,
                1,
                &serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "session/update",
                    "params": {"sequence": 1}
                }),
                false,
            ))
            .unwrap();
        let spill_deadline = Instant::now() + Duration::from_secs(1);
        while !ingress.spill_mode.load(Ordering::Acquire) {
            assert!(
                Instant::now() < spill_deadline,
                "capture worker never entered durable spill mode"
            );
            thread::sleep(Duration::from_millis(1));
        }

        ingress.finish(RelayFinishReason::EditorEof);
        assert!(
            !ingress.flush(Duration::from_millis(100)),
            "barrier acknowledged before the earlier spill and finish were projected"
        );
        assert!(
            ingress.spill.finish_path_for("barrier-order").is_file(),
            "timed-out finalization lost its durable finish marker"
        );
        drop(writer_lock);
        assert!(
            ingress.flush(Duration::from_secs(2)),
            "barrier did not acknowledge after spill replay and finish projection"
        );
        assert!(
            !ingress.spill.finish_path_for("barrier-order").exists(),
            "finish marker remained after terminal projection was acknowledged"
        );
    }

    #[test]
    fn spill_recovery_never_claims_another_live_connection() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("acp-ingress");
        let live = SpillStore::new(dir.clone(), "live-connection".to_string()).unwrap();
        live.append(&capture_frame(
            "live-connection",
            Direction::AgentToClient,
            7,
            &serde_json::json!({"jsonrpc":"2.0","method":"ext/live"}),
            false,
        ))
        .unwrap();

        let recovery = SpillStore::new(dir, "recovery-connection".to_string()).unwrap();
        assert!(
            recovery.take_all().unwrap().is_empty(),
            "recovery worker claimed a spill owned by a live relay"
        );
        drop(live);

        let recovered = recovery.take_all().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].connection_id, "live-connection");
        assert_eq!(recovered[0].sequence, 7);
        recovery.complete_claimed_frames().unwrap();
    }

    #[test]
    fn spill_recovery_preserves_the_terminal_reason_until_acknowledged() {
        let temp = tempfile::tempdir().unwrap();
        let dir = temp.path().join("acp-ingress");
        let live = SpillStore::new(dir.clone(), "finished-connection".to_string()).unwrap();
        live.persist_finish(&RelayFinishReason::AgentError(
            "upstream terminated".to_string(),
        ))
        .unwrap();
        drop(live);

        let recovery = SpillStore::new(dir.clone(), "recovery-connection".to_string()).unwrap();
        assert!(recovery.take_all().unwrap().is_empty());
        assert_eq!(
            recovery.take_recovered_finishes(),
            vec![RelayFinishReason::AgentError(
                "upstream terminated".to_string()
            )]
        );
        assert!(
            fs::read_dir(&dir)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .any(|entry| spill_finish_path(&entry.path())),
            "finish marker disappeared before terminal projection committed"
        );
        recovery.complete_finish().unwrap();
        assert!(
            !fs::read_dir(&dir)
                .unwrap()
                .filter_map(|entry| entry.ok())
                .any(|entry| spill_finish_path(&entry.path())),
            "acknowledged finish marker remained in the journal"
        );
    }
}
