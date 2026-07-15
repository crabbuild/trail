//! Qualified Linux inotify observer.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fs::{self, File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use inotify::{EventMask, Inotify, WatchDescriptor, WatchMask};
use rustix::fs::{fstat, fsync, openat, unlinkat, AtFlags, Mode, OFlags};

use super::{ObserverFence, ObserverLease, QualifiedObserver};
use crate::db::change_ledger::reconcile::{ObserverEvent, ObserverQualification};
#[cfg(debug_assertions)]
use crate::db::change_ledger::{
    begin_reconciliation, install_initial_scan_hook, reconcile_full, BaselineIdentity,
    CompiledPolicy, FilesystemIdentity, PolicyIdentity, ProviderIdentity, ReconcileMode,
    RecordingPolicySnapshot, ScopeIdentity, ScopeKind,
};
use crate::db::change_ledger::{
    DurableCut, EvidenceFlags, EvidenceSource, ExpectedScope, LedgerPath, ObserverRecord,
    ObserverWriterBinding, ProviderCapabilities, ScopeId, SegmentWriter,
};
use crate::error::{Error, Result};
#[cfg(debug_assertions)]
use crate::{InitImportMode, Trail};

const READ_BUFFER_BYTES: usize = 256 * 1024;
const MAX_RETAINED_EVENTS: usize = 65_536;
const MAX_PENDING_RECORDS: usize = 8_192;
const COOKIE_EXPIRY: Duration = Duration::from_millis(75);
const FENCE_TIMEOUT: Duration = Duration::from_secs(10);
const LOOP_PAUSE: Duration = Duration::from_millis(2);

const WATCH_MASK: WatchMask = WatchMask::CREATE
    .union(WatchMask::DELETE)
    .union(WatchMask::MODIFY)
    .union(WatchMask::CLOSE_WRITE)
    .union(WatchMask::ATTRIB)
    .union(WatchMask::MOVED_FROM)
    .union(WatchMask::MOVED_TO)
    .union(WatchMask::DELETE_SELF)
    .union(WatchMask::MOVE_SELF)
    .union(WatchMask::DONT_FOLLOW)
    .union(WatchMask::ONLYDIR)
    .union(WatchMask::EXCL_UNLINK);

pub(crate) trait ObserverDurability: Send {
    fn binding(&self) -> ObserverWriterBinding;
    fn append_and_flush(&mut self, record: ObserverRecord) -> Result<DurableCut>;
}

/// Segment writes run on the observer worker, never in an inotify callback.
/// The direct inotify adapter has no callback that could acquire the workspace
/// lock or primary SQLite connection.
pub(crate) struct SegmentWriterDurability {
    writer: SegmentWriter,
    binding: ObserverWriterBinding,
}

impl SegmentWriterDurability {
    pub(crate) fn new(
        mut writer: SegmentWriter,
        provider_identity: Vec<u8>,
        fence_nonce: Vec<u8>,
    ) -> Result<Self> {
        let binding = writer.bind_native_observer(provider_identity, fence_nonce)?;
        Ok(Self { writer, binding })
    }
}

impl ObserverDurability for SegmentWriterDurability {
    fn binding(&self) -> ObserverWriterBinding {
        self.binding.clone()
    }

    fn append_and_flush(&mut self, record: ObserverRecord) -> Result<DurableCut> {
        self.writer.append(&[record])?;
        self.writer.flush_durable()
    }
}

#[derive(Clone)]
struct DurableEvent {
    event: ObserverEvent,
    cut: DurableCut,
}

struct PendingRename {
    path: LedgerPath,
    is_dir: bool,
    observed_at: Instant,
}

struct State {
    active: bool,
    revoked: Option<String>,
    events: Vec<DurableEvent>,
    next_sequence: u64,
    pending_renames: HashMap<u32, PendingRename>,
    issued_fences: HashMap<Vec<u8>, IssuedFence>,
    fail_next_watch_add: bool,
}

#[derive(Clone)]
enum IssuedFenceKind {
    Start,
    End { start_nonce: Vec<u8> },
}

#[derive(Clone)]
struct IssuedFence {
    public: ObserverFence,
    expected: ExpectedScope,
    root_identity: Vec<u8>,
    owner_token: String,
    provider_id: String,
    provider_identity: Vec<u8>,
    owner_fence_nonce: Vec<u8>,
    sentinel_path: LedgerPath,
    create_sequence: u64,
    delete_sequence: u64,
    segment_id: String,
    durable_cut: DurableCut,
    kind: IssuedFenceKind,
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
    shutdown: AtomicBool,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn revoke(&self, reason: impl Into<String>) {
        let mut state = self.lock();
        if state.revoked.is_none() {
            state.revoked = Some(reason.into());
        }
        state.active = false;
        self.changed.notify_all();
    }
}

pub(crate) struct LinuxInotifyObserver {
    root_path: PathBuf,
    root: File,
    root_identity: Vec<u8>,
    provider_identity: Vec<u8>,
    provider_id: String,
    owner_token: String,
    owner_fence_nonce: Vec<u8>,
    shared: Arc<Shared>,
    workers: Mutex<Vec<JoinHandle<()>>>,
}

struct PlannedRecord {
    path: LedgerPath,
    flags: EvidenceFlags,
}

enum DurabilityCommand {
    Record(PlannedRecord),
}

impl LinuxInotifyObserver {
    pub(crate) fn start(root_path: &Path, durability: Box<dyn ObserverDurability>) -> Result<Self> {
        let root = open_root_no_follow(root_path)?;
        let root_identity = root_identity(&root)?;
        let binding = durability.binding();
        if binding.owner_token.is_empty()
            || binding.provider_id.is_empty()
            || binding.provider_identity.is_empty()
            || binding.fence_nonce.len() < 16
            || binding.provider_id != hex::encode(&binding.provider_identity)
        {
            return Err(Error::InvalidInput(
                "native observer durability binding is incomplete or inconsistent".into(),
            ));
        }
        let mut inotify = Inotify::init()?;
        let mut watches = HashMap::new();
        add_tree(&mut inotify, root_path, Path::new(""), &mut watches, false)?;

        let owner_token = binding.owner_token.clone();
        let provider_id = binding.provider_id.clone();
        let provider_identity = binding.provider_identity.clone();
        let owner_fence_nonce = binding.fence_nonce.clone();
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                active: true,
                revoked: None,
                events: Vec::new(),
                next_sequence: 1,
                pending_renames: HashMap::new(),
                issued_fences: HashMap::new(),
                fail_next_watch_add: false,
            }),
            changed: Condvar::new(),
            shutdown: AtomicBool::new(false),
        });
        let worker_shared = Arc::clone(&shared);
        let worker_root_path = root_path.to_path_buf();
        let worker_root = root.try_clone()?;
        let expected_identity = root_identity.clone();
        let (records_tx, records_rx) = mpsc::sync_channel(MAX_PENDING_RECORDS);
        let durability_shared = Arc::clone(&shared);
        let durability_worker = thread::Builder::new()
            .name("trail-linux-observer-durability".into())
            .spawn(move || run_durability_worker(records_rx, durability, durability_shared))?;
        let worker = thread::Builder::new()
            .name("trail-linux-inotify".into())
            .spawn(move || {
                run_worker(
                    inotify,
                    watches,
                    worker_root_path,
                    worker_root,
                    expected_identity,
                    records_tx,
                    worker_shared,
                )
            })?;
        Ok(Self {
            root_path: root_path.to_path_buf(),
            root,
            root_identity,
            provider_identity,
            provider_id,
            owner_token,
            owner_fence_nonce,
            shared,
            workers: Mutex::new(vec![worker, durability_worker]),
        })
    }

    pub(crate) fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            durable_cursor: true,
            linearizable_fence: true,
            rename_pairing: true,
            overflow_scope: true,
            filesystem_supported: true,
            clean_proof_allowed: true,
            power_loss_durability: true,
        }
    }

    pub(crate) fn root_identity(&self) -> Result<Vec<u8>> {
        self.ensure_available()?;
        if root_identity(&self.root)? != self.root_identity
            || root_identity(&open_root_no_follow(&self.root_path)?)? != self.root_identity
        {
            self.shared.revoke("inotify_root_replaced");
            return Err(reconcile_error("inotify_root_replaced"));
        }
        Ok(self.root_identity.clone())
    }

    pub(crate) fn lease(&self) -> Result<ObserverLease> {
        Ok(ObserverLease {
            owner_token: self.owner_token.clone(),
            root_identity: self.root_identity()?,
            capabilities: self.capabilities(),
        })
    }

    fn ensure_available(&self) -> Result<()> {
        let state = self.shared.lock();
        if let Some(reason) = &state.revoked {
            return Err(reconcile_error(reason));
        }
        if !state.active {
            return Err(reconcile_error("inotify_observer_unavailable"));
        }
        Ok(())
    }

    fn sentinel_fence(
        &self,
        expected: &ExpectedScope,
        kind: IssuedFenceKind,
    ) -> Result<ObserverFence> {
        self.ensure_available()?;
        self.root_identity()?;
        if expected.provider_identity != self.provider_identity {
            self.shared.revoke("inotify_provider_identity_mismatch");
            return Err(reconcile_error("inotify_provider_identity_mismatch"));
        }
        let mut nonce = [0_u8; 24];
        getrandom::getrandom(&mut nonce).map_err(|error| {
            Error::InvalidInput(format!("observer fence nonce failed: {error}"))
        })?;
        let nonce_hex = hex::encode(nonce);
        let name = format!(".trail-observer-fence-{nonce_hex}");
        let path = LedgerPath::parse(&name)?;

        let fd = openat(
            &self.root,
            Path::new(&name),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        )
        .map_err(|error| Error::Io(error.into()))?;
        let mut sentinel = File::from(fd);
        sentinel.write_all(nonce_hex.as_bytes())?;
        sentinel.sync_all()?;
        fsync(&self.root).map_err(|error| Error::Io(error.into()))?;
        let create = self.wait_for(&path, EvidenceFlags::CREATE, 0)?;

        unlinkat(&self.root, Path::new(&name), AtFlags::empty())
            .map_err(|error| Error::Io(error.into()))?;
        fsync(&self.root).map_err(|error| Error::Io(error.into()))?;
        let delete = self.wait_for(&path, EvidenceFlags::DELETE, create.event.sequence)?;
        let public = ObserverFence {
            sequence: delete.event.sequence,
            durable_offset: delete.cut.durable_end_offset,
            nonce: nonce.to_vec(),
        };
        let issued = IssuedFence {
            public: public.clone(),
            expected: expected.clone(),
            root_identity: self.root_identity.clone(),
            owner_token: self.owner_token.clone(),
            provider_id: self.provider_id.clone(),
            provider_identity: self.provider_identity.clone(),
            owner_fence_nonce: self.owner_fence_nonce.clone(),
            sentinel_path: path,
            create_sequence: create.event.sequence,
            delete_sequence: delete.event.sequence,
            segment_id: delete.cut.segment_id.clone(),
            durable_cut: delete.cut,
            kind,
        };
        self.shared
            .lock()
            .issued_fences
            .insert(nonce.to_vec(), issued);
        Ok(public)
    }

    fn issued_fence(&self, expected: &ExpectedScope, fence: &ObserverFence) -> Result<IssuedFence> {
        let state = self.shared.lock();
        let Some(issued) = state.issued_fences.get(&fence.nonce) else {
            drop(state);
            self.shared.revoke("inotify_fence_unknown_or_replayed");
            return Err(reconcile_error("inotify_fence_unknown_or_replayed"));
        };
        let exact = issued.public == *fence
            && issued.expected == *expected
            && issued.root_identity == self.root_identity
            && issued.owner_token == self.owner_token
            && issued.provider_id == self.provider_id
            && issued.provider_identity == self.provider_identity
            && issued.owner_fence_nonce == self.owner_fence_nonce
            && issued.delete_sequence == fence.sequence
            && issued.durable_cut.last_sequence == fence.sequence
            && issued.durable_cut.durable_end_offset == fence.durable_offset
            && issued.durable_cut.segment_id == issued.segment_id
            && issued.create_sequence < issued.delete_sequence
            && issued.sentinel_path.as_str()
                == format!(".trail-observer-fence-{}", hex::encode(&fence.nonce));
        if !exact {
            drop(state);
            self.shared.revoke("inotify_fence_authentication_mismatch");
            return Err(reconcile_error("inotify_fence_authentication_mismatch"));
        }
        Ok(issued.clone())
    }

    fn wait_for(
        &self,
        path: &LedgerPath,
        required: EvidenceFlags,
        after: u64,
    ) -> Result<DurableEvent> {
        let deadline = Instant::now() + FENCE_TIMEOUT;
        let mut state = self.shared.lock();
        loop {
            if let Some(reason) = &state.revoked {
                return Err(reconcile_error(reason));
            }
            if let Some(found) = state.events.iter().find(|item| {
                item.event.sequence > after
                    && item.event.path == *path
                    && item.event.flags.0 & required.0 == required.0
            }) {
                return Ok(found.clone());
            }
            let now = Instant::now();
            if now >= deadline {
                drop(state);
                self.shared.revoke("inotify_fence_delivery_timeout");
                return Err(reconcile_error("inotify_fence_delivery_timeout"));
            }
            let duration = deadline.saturating_duration_since(now);
            let waited = self
                .shared
                .changed
                .wait_timeout(state, duration)
                .unwrap_or_else(|poison| poison.into_inner());
            state = waited.0;
        }
    }

    fn shutdown_inner(&self) -> Result<()> {
        self.shared.shutdown.store(true, Ordering::Release);
        self.shared.changed.notify_all();
        let workers = std::mem::take(
            &mut *self
                .workers
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()),
        );
        for worker in workers {
            worker
                .join()
                .map_err(|_| Error::InvalidInput("inotify observer worker panicked".into()))?;
        }
        let mut state = self.shared.lock();
        state.active = false;
        Ok(())
    }

    #[cfg(debug_assertions)]
    fn test_fail_next_watch_add(&self) {
        self.shared.lock().fail_next_watch_add = true;
    }
}

impl QualifiedObserver for LinuxInotifyObserver {
    fn begin_observation(&self, expected: &ExpectedScope) -> Result<ObserverFence> {
        self.sentinel_fence(expected, IssuedFenceKind::Start)
    }

    fn end_fence(&self, expected: &ExpectedScope, start: &ObserverFence) -> Result<ObserverFence> {
        if expected.provider_identity != self.provider_identity || start.nonce.is_empty() {
            return Err(reconcile_error(
                "inotify_reconciliation_start_not_qualified",
            ));
        }
        let issued_start = self.issued_fence(expected, start)?;
        if !matches!(issued_start.kind, IssuedFenceKind::Start) {
            self.shared
                .revoke("inotify_reconciliation_start_not_qualified");
            return Err(reconcile_error(
                "inotify_reconciliation_start_not_qualified",
            ));
        }
        let end = self.sentinel_fence(
            expected,
            IssuedFenceKind::End {
                start_nonce: start.nonce.clone(),
            },
        )?;
        if end.sequence <= start.sequence || end.durable_offset < start.durable_offset {
            self.shared.revoke("inotify_non_monotonic_fence");
            return Err(reconcile_error("inotify_non_monotonic_fence"));
        }
        Ok(end)
    }

    fn drain_through(
        &self,
        expected: &ExpectedScope,
        root_handle_identity: &[u8],
        start: &ObserverFence,
        end: &ObserverFence,
        sink: &mut dyn FnMut(ObserverEvent) -> Result<()>,
    ) -> Result<ObserverQualification> {
        self.ensure_available()?;
        if self.root_identity()? != root_handle_identity {
            self.shared.revoke("inotify_root_identity_mismatch");
            return Err(reconcile_error("inotify_root_identity_mismatch"));
        }
        let issued_start = self.issued_fence(expected, start)?;
        let issued_end = self.issued_fence(expected, end)?;
        if !matches!(issued_start.kind, IssuedFenceKind::Start)
            || !matches!(
                &issued_end.kind,
                IssuedFenceKind::End { start_nonce } if *start_nonce == start.nonce
            )
        {
            self.shared.revoke("inotify_fence_interval_mismatch");
            return Err(reconcile_error("inotify_fence_interval_mismatch"));
        }
        let (events, end_cut) = {
            let state = self.shared.lock();
            let events = state
                .events
                .iter()
                .filter(|item| {
                    item.event.sequence > start.sequence && item.event.sequence <= end.sequence
                })
                .map(|item| item.event.clone())
                .collect::<Vec<_>>();
            let end_cut = state
                .events
                .iter()
                .find(|item| item.event.sequence == end.sequence)
                .map(|item| item.cut.clone());
            (events, end_cut)
        };
        for event in events {
            sink(event)?;
        }
        let end_cut = end_cut.ok_or_else(|| reconcile_error("inotify_end_fence_not_retained"))?;
        if end_cut != issued_end.durable_cut {
            self.shared.revoke("inotify_end_fence_durable_cut_mismatch");
            return Err(reconcile_error("inotify_end_fence_durable_cut_mismatch"));
        }
        let qualification = ObserverQualification::native(
            expected,
            root_handle_identity.to_vec(),
            start.clone(),
            end.clone(),
            self.owner_token.clone(),
            self.owner_fence_nonce.clone(),
            end_cut.segment_id,
            end_cut.durable_end_offset,
            end_cut.durable_end_offset,
        );
        let mut state = self.shared.lock();
        state
            .events
            .retain(|item| item.event.sequence > end.sequence);
        state.issued_fences.remove(&start.nonce);
        state.issued_fences.remove(&end.nonce);
        Ok(qualification)
    }
}

impl Drop for LinuxInotifyObserver {
    fn drop(&mut self) {
        let _ = self.shutdown_inner();
    }
}

fn run_worker(
    mut inotify: Inotify,
    mut watches: HashMap<WatchDescriptor, PathBuf>,
    root_path: PathBuf,
    root: File,
    root_identity_expected: Vec<u8>,
    records: SyncSender<DurabilityCommand>,
    shared: Arc<Shared>,
) {
    let mut buffer = vec![0_u8; READ_BUFFER_BYTES];
    while !shared.shutdown.load(Ordering::Acquire) {
        if shared.lock().revoked.is_some() {
            break;
        }
        if verify_root(&root_path, &root, &root_identity_expected).is_err() {
            shared.revoke("inotify_root_replaced");
            break;
        }
        let events = match inotify.read_events(&mut buffer) {
            Ok(events) => events
                .map(|event| {
                    (
                        event.wd,
                        event.mask,
                        event.cookie,
                        event.name.map(OsStr::to_os_string),
                    )
                })
                .collect::<Vec<_>>(),
            Err(error) if error.kind() == ErrorKind::WouldBlock => Vec::new(),
            Err(_) => {
                shared.revoke("inotify_decode_or_read_failure");
                break;
            }
        };
        for (wd, mask, cookie, name) in events {
            if classify_raw_authority_event(&shared, mask, watches.contains_key(&wd)).is_err() {
                break;
            }
            let Some(parent) = watches.get(&wd).cloned() else {
                break;
            };
            if parent.as_os_str().is_empty()
                && (mask.contains(EventMask::DELETE_SELF) || mask.contains(EventMask::MOVE_SELF))
            {
                shared.revoke("inotify_root_deleted_or_moved");
                break;
            }
            let Some(name) = name else {
                continue;
            };
            let Some(name) = name.to_str() else {
                shared.revoke("inotify_path_decode_ambiguity");
                break;
            };
            if name.is_empty() || name == "." || name == ".." || name.contains('/') {
                shared.revoke("inotify_path_decode_ambiguity");
                break;
            }
            let relative = parent.join(name);
            let Some(relative_text) = relative.to_str() else {
                shared.revoke("inotify_path_decode_ambiguity");
                break;
            };
            let path = match LedgerPath::parse(relative_text) {
                Ok(path) => path,
                Err(_) => {
                    shared.revoke("inotify_path_decode_ambiguity");
                    break;
                }
            };
            let is_dir = mask.contains(EventMask::ISDIR);
            if is_dir && (mask.contains(EventMask::CREATE) || mask.contains(EventMask::MOVED_TO)) {
                let fail = {
                    let mut state = shared.lock();
                    let fail = state.fail_next_watch_add;
                    state.fail_next_watch_add = false;
                    fail
                };
                if add_tree(&mut inotify, &root_path, &relative, &mut watches, fail).is_err() {
                    // A just-created directory can be renamed again before its
                    // CREATE is drained. If the old endpoint is already gone,
                    // the complete parent prefix plus the later MOVED_TO
                    // watch-before-enumerate closes that race. Every other
                    // watch-add failure revokes continuity globally.
                    if mask.contains(EventMask::CREATE) && !root_path.join(&relative).exists() {
                        if enqueue(
                            &shared,
                            &records,
                            complete_parent(&path),
                            EvidenceFlags::PROVIDER_COMPLETE_PREFIX,
                        )
                        .is_err()
                        {
                            break;
                        }
                    } else {
                        shared.revoke("inotify_watch_add_failure");
                        break;
                    }
                }
                if enqueue(
                    &shared,
                    &records,
                    complete_parent(&path),
                    EvidenceFlags::PROVIDER_COMPLETE_PREFIX,
                )
                .is_err()
                {
                    break;
                }
            }
            let flags = event_flags(mask);
            if flags.0 != 0 && enqueue(&shared, &records, path.clone(), flags).is_err() {
                break;
            }
            if mask.contains(EventMask::MOVED_FROM) && cookie != 0 {
                shared.lock().pending_renames.insert(
                    cookie,
                    PendingRename {
                        path: path.clone(),
                        is_dir,
                        observed_at: Instant::now(),
                    },
                );
            }
            if mask.contains(EventMask::MOVED_TO) && cookie != 0 {
                let paired = shared.lock().pending_renames.remove(&cookie);
                if let Some(from) = paired {
                    if from.is_dir && is_dir {
                        if enqueue(
                            &shared,
                            &records,
                            from.path.clone(),
                            EvidenceFlags::PROVIDER_COMPLETE_PREFIX,
                        )
                        .is_err()
                        {
                            break;
                        }
                        remap_watches(&mut watches, Path::new(from.path.as_str()), &relative);
                    }
                }
            }
        }
        if expire_rename_cookies(&shared, &records).is_err() {
            break;
        }
        thread::sleep(LOOP_PAUSE);
    }
    let mut state = shared.lock();
    state.active = false;
    shared.changed.notify_all();
}

fn classify_raw_authority_event(
    shared: &Shared,
    mask: EventMask,
    known_watch_descriptor: bool,
) -> Result<()> {
    if mask.contains(EventMask::Q_OVERFLOW) {
        shared.revoke("inotify_queue_overflow");
        return Err(reconcile_error("inotify_queue_overflow"));
    }
    if mask.contains(EventMask::IGNORED) {
        shared.revoke("inotify_watch_ignored");
        return Err(reconcile_error("inotify_watch_ignored"));
    }
    if !known_watch_descriptor {
        shared.revoke("inotify_unknown_watch_descriptor");
        return Err(reconcile_error("inotify_unknown_watch_descriptor"));
    }
    Ok(())
}

fn enqueue(
    shared: &Shared,
    records: &SyncSender<DurabilityCommand>,
    path: LedgerPath,
    flags: EvidenceFlags,
) -> Result<()> {
    match records.try_send(DurabilityCommand::Record(PlannedRecord { path, flags })) {
        Ok(()) => Ok(()),
        Err(TrySendError::Full(_)) => {
            shared.revoke("inotify_bounded_queue_overflow");
            Err(reconcile_error("inotify_bounded_queue_overflow"))
        }
        Err(TrySendError::Disconnected(_)) => {
            shared.revoke("inotify_durability_worker_unavailable");
            Err(reconcile_error("inotify_durability_worker_unavailable"))
        }
    }
}

fn run_durability_worker(
    records: Receiver<DurabilityCommand>,
    mut durability: Box<dyn ObserverDurability>,
    shared: Arc<Shared>,
) {
    loop {
        if shared.shutdown.load(Ordering::Acquire) {
            break;
        }
        match records.recv_timeout(Duration::from_millis(10)) {
            Ok(command) => match command {
                DurabilityCommand::Record(record) => {
                    if persist(&shared, durability.as_mut(), record.path, record.flags).is_err() {
                        break;
                    }
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) if !shared.shutdown.load(Ordering::Acquire) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => break,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    shared.changed.notify_all();
}

fn persist(
    shared: &Shared,
    durability: &mut dyn ObserverDurability,
    path: LedgerPath,
    flags: EvidenceFlags,
) -> Result<()> {
    let sequence = {
        let mut state = shared.lock();
        if state.events.len() >= MAX_RETAINED_EVENTS {
            drop(state);
            shared.revoke("inotify_bounded_queue_overflow");
            return Err(reconcile_error("inotify_bounded_queue_overflow"));
        }
        let sequence = state.next_sequence;
        state.next_sequence = state.next_sequence.saturating_add(1);
        sequence
    };
    let cut = match durability.append_and_flush(ObserverRecord {
        sequence,
        source: EvidenceSource::Observer,
        path: path.clone(),
        flags,
        provider_cursor: sequence.to_be_bytes().to_vec(),
    }) {
        Ok(cut) if cut.last_sequence == sequence => cut,
        Ok(_) => {
            shared.revoke("inotify_durability_cut_mismatch");
            return Err(reconcile_error("inotify_durability_cut_mismatch"));
        }
        Err(_) => {
            shared.revoke("inotify_durability_failure");
            return Err(reconcile_error("inotify_durability_failure"));
        }
    };
    let mut state = shared.lock();
    state.events.push(DurableEvent {
        event: ObserverEvent {
            path,
            flags,
            sequence,
        },
        cut,
    });
    shared.changed.notify_all();
    Ok(())
}

fn expire_rename_cookies(shared: &Shared, records: &SyncSender<DurabilityCommand>) -> Result<()> {
    let expired = {
        let mut state = shared.lock();
        let now = Instant::now();
        let cookies = state
            .pending_renames
            .iter()
            .filter_map(|(cookie, pending)| {
                (now.duration_since(pending.observed_at) >= COOKIE_EXPIRY).then_some(*cookie)
            })
            .collect::<Vec<_>>();
        cookies
            .into_iter()
            .filter_map(|cookie| state.pending_renames.remove(&cookie))
            .collect::<Vec<_>>()
    };
    for pending in expired {
        enqueue(
            shared,
            records,
            complete_parent(&pending.path),
            EvidenceFlags::PROVIDER_COMPLETE_PREFIX,
        )?;
    }
    Ok(())
}

fn add_tree(
    inotify: &mut Inotify,
    root: &Path,
    relative: &Path,
    watches: &mut HashMap<WatchDescriptor, PathBuf>,
    inject_failure: bool,
) -> Result<()> {
    if observer_internal_path(relative) {
        return Ok(());
    }
    if inject_failure {
        return Err(Error::InvalidInput("injected watch-add failure".into()));
    }
    let absolute = root.join(relative);
    let metadata = fs::symlink_metadata(&absolute)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(Error::InvalidInput(
            "inotify recursive watch target is not a no-follow directory".into(),
        ));
    }
    #[allow(deprecated)]
    let wd = inotify.add_watch(&absolute, WATCH_MASK)?;
    watches.insert(wd, relative.to_path_buf());
    let entries = fs::read_dir(&absolute)?.collect::<std::io::Result<Vec<_>>>()?;
    for entry in entries {
        let metadata = entry.file_type()?;
        if metadata.is_dir() && !metadata.is_symlink() {
            let child = relative.join(entry.file_name());
            if !observer_internal_path(&child) {
                add_tree(inotify, root, &child, watches, false)?;
            }
        }
    }
    Ok(())
}

fn observer_internal_path(relative: &Path) -> bool {
    relative.components().next().is_some_and(|component| {
        component.as_os_str() == ".trail" || component.as_os_str() == ".git"
    })
}

fn remap_watches(watches: &mut HashMap<WatchDescriptor, PathBuf>, from: &Path, to: &Path) {
    for relative in watches.values_mut() {
        if let Ok(suffix) = relative.strip_prefix(from) {
            *relative = to.join(suffix);
        }
    }
}

fn event_flags(mask: EventMask) -> EvidenceFlags {
    let mut flags = EvidenceFlags::default();
    if mask.contains(EventMask::CREATE) {
        flags |= EvidenceFlags::CREATE;
    }
    if mask.intersects(EventMask::MODIFY | EventMask::CLOSE_WRITE) {
        flags |= EvidenceFlags::CONTENT;
    }
    if mask.contains(EventMask::ATTRIB) {
        flags |= EvidenceFlags::MODE;
    }
    if mask.contains(EventMask::DELETE) {
        flags |= EvidenceFlags::DELETE;
    }
    if mask.contains(EventMask::MOVED_FROM) {
        flags |= EvidenceFlags::RENAME_FROM;
    }
    if mask.contains(EventMask::MOVED_TO) {
        flags |= EvidenceFlags::RENAME_TO;
    }
    flags
}

fn complete_parent(path: &LedgerPath) -> LedgerPath {
    let text = path.as_str();
    match text.rsplit_once('/') {
        Some((parent, _)) => LedgerPath::parse(parent).unwrap_or_else(|_| path.clone()),
        None => path.clone(),
    }
}

fn open_root_no_follow(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?)
}

fn root_identity(file: &File) -> Result<Vec<u8>> {
    let stat = fstat(file).map_err(|error| Error::Io(error.into()))?;
    Ok(format!(
        "root-v1:dev={};ino={};mode={};uid={};gid={}",
        stat.st_dev, stat.st_ino, stat.st_mode, stat.st_uid, stat.st_gid
    )
    .into_bytes())
}

fn verify_root(path: &Path, root: &File, expected: &[u8]) -> Result<()> {
    if root_identity(root)? != expected || root_identity(&open_root_no_follow(path)?)? != expected {
        return Err(reconcile_error("inotify_root_replaced"));
    }
    Ok(())
}

fn reconcile_error(reason: &str) -> Error {
    Error::ChangeLedgerReconcileRequired {
        scope: "native-linux-observer".into(),
        state: "untrusted_gap".into(),
        reason: reason.into(),
        command: "trail status".into(),
    }
}

#[cfg(debug_assertions)]
struct MemoryDurability {
    offset: u64,
    fail_after: Option<u64>,
    binding: ObserverWriterBinding,
}

#[cfg(debug_assertions)]
impl ObserverDurability for MemoryDurability {
    fn binding(&self) -> ObserverWriterBinding {
        self.binding.clone()
    }

    fn append_and_flush(&mut self, record: ObserverRecord) -> Result<DurableCut> {
        if self.fail_after == Some(self.offset) {
            return Err(Error::InvalidInput(
                "injected observer durability failure".into(),
            ));
        }
        self.offset = self.offset.saturating_add(1);
        Ok(DurableCut {
            segment_id: "linux-native-test".into(),
            durable_end_offset: self.offset,
            last_sequence: record.sequence,
            last_hash: [0; 32],
            provider_cursor: record.provider_cursor,
        })
    }
}

#[cfg(debug_assertions)]
fn memory_durability(fail_after: Option<u64>) -> MemoryDurability {
    let mut owner = [0_u8; 32];
    let mut fence = [0_u8; 24];
    getrandom::getrandom(&mut owner).expect("test observer owner entropy");
    getrandom::getrandom(&mut fence).expect("test observer fence entropy");
    let provider_identity = b"linux-inotify-memory-test-v1".to_vec();
    MemoryDurability {
        offset: 0,
        fail_after,
        binding: ObserverWriterBinding {
            owner_token: hex::encode(owner),
            provider_id: hex::encode(&provider_identity),
            provider_identity,
            fence_nonce: fence.to_vec(),
        },
    }
}

#[cfg(debug_assertions)]
struct NativeFixture {
    _temp: tempfile::TempDir,
    db: Trail,
    expected: ExpectedScope,
    policy: CompiledPolicy,
    segment_directory: PathBuf,
}

#[cfg(debug_assertions)]
impl NativeFixture {
    fn new(setup: impl FnOnce(&Path) -> Result<()>) -> Result<Self> {
        let temp = tempfile::tempdir()?;
        setup(temp.path())?;
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false)?;
        let db = Trail::open(temp.path())?;
        let branch = db.current_branch()?;
        let head = db.resolve_branch_ref(&branch)?;
        let scope = ScopeIdentity {
            scope_id: ScopeId([0x91; 32]),
            kind: ScopeKind::Workspace,
            owner_id: "linux-native-reconciliation".into(),
        };
        let fingerprint = [0x92; 32];
        let filesystem_identity = root_identity(&open_root_no_follow(temp.path())?)?;
        let provider_identity = b"linux-inotify-native-v1".to_vec();
        let baseline = BaselineIdentity {
            ref_name: head.name.clone(),
            ref_generation: u64::try_from(head.generation)
                .map_err(|_| Error::Corrupt("negative native ref generation".into()))?,
            change_id: head.change_id,
            root_id: head.root_id,
        };
        db.changed_path_ledger().begin_scope(
            &scope,
            &baseline,
            &PolicyIdentity {
                fingerprint,
                generation: 1,
            },
            &FilesystemIdentity(filesystem_identity.clone()),
            &ProviderIdentity {
                identity: provider_identity.clone(),
                capabilities: ProviderCapabilities {
                    durable_cursor: true,
                    linearizable_fence: true,
                    rename_pairing: true,
                    overflow_scope: true,
                    filesystem_supported: true,
                    clean_proof_allowed: true,
                    power_loss_durability: true,
                },
            },
        )?;
        let expected = ExpectedScope {
            scope_id: scope.scope_id,
            epoch: 1,
            ref_name: baseline.ref_name,
            ref_generation: baseline.ref_generation,
            baseline_root: baseline.root_id,
            policy_fingerprint: fingerprint,
            policy_generation: 1,
            filesystem_identity,
            provider_identity,
        };
        let policy = CompiledPolicy::for_reconciliation_test(
            RecordingPolicySnapshot {
                workspace_root: db.workspace_root.clone(),
                ignore_gitignored: true,
                dependency_files: Vec::new(),
                case_sensitive: true,
                rule_sources: Vec::new(),
            },
            fingerprint,
            &expected,
        );
        let segment_directory = db.db_dir.join("change-observer-segments");
        Ok(Self {
            _temp: temp,
            db,
            expected,
            policy,
            segment_directory,
        })
    }

    fn observer(&self) -> Result<LinuxInotifyObserver> {
        let mut owner = [0_u8; 32];
        let mut fence = [0_u8; 24];
        getrandom::getrandom(&mut owner).map_err(|error| Error::InvalidInput(error.to_string()))?;
        getrandom::getrandom(&mut fence).map_err(|error| Error::InvalidInput(error.to_string()))?;
        let writer = SegmentWriter::acquire(
            &self.db.sqlite_path,
            &self.segment_directory,
            self.expected.scope_id,
            self.expected.epoch,
            owner,
            &hex::encode(&self.expected.provider_identity),
            Vec::new(),
            Duration::from_secs(3_600),
        )?;
        let durability = SegmentWriterDurability::new(
            writer,
            self.expected.provider_identity.clone(),
            fence.to_vec(),
        )?;
        LinuxInotifyObserver::start(&self.db.workspace_root, Box::new(durability))
    }

    fn published_paths(&self) -> Result<Vec<String>> {
        let mut statement = self.db.conn.prepare(
            "SELECT normalized_path FROM changed_path_entries
             WHERE scope_id=?1 ORDER BY normalized_path COLLATE BINARY",
        )?;
        let paths = statement
            .query_map([self.expected.scope_id.to_text()], |row| row.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(paths)
    }
}

#[cfg(debug_assertions)]
struct SlowDurability {
    inner: MemoryDurability,
}

#[cfg(debug_assertions)]
impl ObserverDurability for SlowDurability {
    fn binding(&self) -> ObserverWriterBinding {
        self.inner.binding()
    }

    fn append_and_flush(&mut self, record: ObserverRecord) -> Result<DurableCut> {
        thread::sleep(Duration::from_millis(2));
        self.inner.append_and_flush(record)
    }
}

#[cfg(debug_assertions)]
fn fixture() -> std::result::Result<(tempfile::TempDir, LinuxInotifyObserver), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let observer = LinuxInotifyObserver::start(temp.path(), Box::new(memory_durability(None)))
        .map_err(|error| error.to_string())?;
    Ok((temp, observer))
}

#[cfg(debug_assertions)]
fn expected_for(observer: &LinuxInotifyObserver, scope_byte: u8) -> ExpectedScope {
    ExpectedScope {
        scope_id: ScopeId([scope_byte; 32]),
        epoch: 1,
        ref_name: "refs/branches/main".into(),
        ref_generation: 1,
        baseline_root: crate::ObjectId(format!("object_linux_observer_{scope_byte}")),
        policy_fingerprint: [8; 32],
        policy_generation: 1,
        filesystem_identity: observer.root_identity.clone(),
        provider_identity: observer.provider_identity.clone(),
    }
}

#[cfg(debug_assertions)]
fn events_through(
    observer: &LinuxInotifyObserver,
) -> std::result::Result<Vec<ObserverEvent>, String> {
    observer
        .begin_observation(&expected_for(observer, 1))
        .map_err(|error| error.to_string())?;
    Ok(observer
        .shared
        .lock()
        .events
        .iter()
        .map(|item| item.event.clone())
        .collect())
}

#[cfg(debug_assertions)]
pub(crate) fn run_recursive_coverage() -> std::result::Result<(), String> {
    let (temp, observer) = fixture()?;
    fs::create_dir(temp.path().join("a")).map_err(|error| error.to_string())?;
    fs::create_dir(temp.path().join("a/b")).map_err(|error| error.to_string())?;
    fs::write(temp.path().join("a/b/file"), b"covered").map_err(|error| error.to_string())?;
    let events = events_through(&observer)?;
    if !events.iter().any(|event| {
        event.path.as_str() == "a" && event.flags.0 & EvidenceFlags::PROVIDER_COMPLETE_PREFIX.0 != 0
    }) {
        return Err("recursive directory add did not emit a complete dirty prefix".into());
    }
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn run_reconciliation_interval_qualification() -> std::result::Result<(), String> {
    let (temp, observer) = fixture()?;
    let expected = expected_for(&observer, 7);
    let start = observer
        .begin_observation(&expected)
        .map_err(|error| error.to_string())?;
    fs::write(temp.path().join("during-reconcile"), b"changed")
        .map_err(|error| error.to_string())?;
    let end = observer
        .end_fence(&expected, &start)
        .map_err(|error| error.to_string())?;
    let mut drained = Vec::new();
    observer
        .drain_through(
            &expected,
            &observer.root_identity,
            &start,
            &end,
            &mut |event| {
                drained.push(event);
                Ok(())
            },
        )
        .map_err(|error| error.to_string())?;
    if !has_event(&drained, "during-reconcile", EvidenceFlags::CREATE) {
        return Err("qualified reconciliation interval omitted an observed change".into());
    }
    Ok(())
}

#[cfg(debug_assertions)]
fn has_event(events: &[ObserverEvent], path: &str, flag: EvidenceFlags) -> bool {
    events
        .iter()
        .any(|event| event.path.as_str() == path && event.flags.0 & flag.0 != 0)
}

#[cfg(debug_assertions)]
fn expect_revoked(
    observer: &LinuxInotifyObserver,
    reason: &str,
) -> std::result::Result<(), String> {
    let deadline = Instant::now() + FENCE_TIMEOUT;
    loop {
        match observer.ensure_available() {
            Err(error) if error.to_string().contains(reason) => return Ok(()),
            Err(error) => return Err(format!("expected {reason}, got {error}")),
            Ok(()) if Instant::now() < deadline => thread::sleep(LOOP_PAUSE),
            Ok(()) => return Err(format!("observer was not revoked for {reason}")),
        }
    }
}

#[cfg(debug_assertions)]
pub(crate) fn run_content_mode_create_delete() -> std::result::Result<(), String> {
    use std::os::unix::fs::PermissionsExt;

    let (temp, observer) = fixture()?;
    let path = temp.path().join("tracked.txt");
    fs::write(&path, b"one").map_err(|error| error.to_string())?;
    fs::write(&path, b"two").map_err(|error| error.to_string())?;
    fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
        .map_err(|error| error.to_string())?;
    fs::remove_file(&path).map_err(|error| error.to_string())?;
    let events = events_through(&observer)?;
    for flag in [
        EvidenceFlags::CREATE,
        EvidenceFlags::CONTENT,
        EvidenceFlags::MODE,
        EvidenceFlags::DELETE,
    ] {
        if !has_event(&events, "tracked.txt", flag) {
            return Err(format!("missing {:?} evidence for tracked.txt", flag));
        }
    }
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn run_rename_matrix() -> std::result::Result<(), String> {
    let (temp, observer) = fixture()?;
    fs::write(temp.path().join("file-a"), b"file").map_err(|error| error.to_string())?;
    fs::rename(temp.path().join("file-a"), temp.path().join("file-b"))
        .map_err(|error| error.to_string())?;
    fs::create_dir(temp.path().join("dir-a")).map_err(|error| error.to_string())?;
    fs::write(temp.path().join("dir-a/child"), b"child").map_err(|error| error.to_string())?;
    fs::rename(temp.path().join("dir-a"), temp.path().join("dir-b"))
        .map_err(|error| error.to_string())?;
    fs::write(temp.path().join("case"), b"case").map_err(|error| error.to_string())?;
    fs::rename(temp.path().join("case"), temp.path().join("CASE"))
        .map_err(|error| error.to_string())?;
    fs::write(temp.path().join("dir-b/after"), b"after").map_err(|error| error.to_string())?;
    let events = events_through(&observer)?;
    for (path, flag) in [
        ("file-a", EvidenceFlags::RENAME_FROM),
        ("file-b", EvidenceFlags::RENAME_TO),
        ("dir-a", EvidenceFlags::RENAME_FROM),
        ("dir-b", EvidenceFlags::RENAME_TO),
        ("case", EvidenceFlags::RENAME_FROM),
        ("CASE", EvidenceFlags::RENAME_TO),
    ] {
        if !has_event(&events, path, flag) {
            return Err(format!("missing rename endpoint {path}"));
        }
    }
    if !events
        .iter()
        .any(|event| event.path.as_str() == "dir-b/after")
        && !has_event(&events, "dir-b", EvidenceFlags::PROVIDER_COMPLETE_PREFIX)
    {
        return Err("directory rename was covered by neither remapped watch nor prefix".into());
    }
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn run_rename_storm_and_cookie_expiry() -> std::result::Result<(), String> {
    let (temp, observer) = fixture()?;
    fs::write(temp.path().join("storm-0"), b"storm").map_err(|error| error.to_string())?;
    for index in 0..128 {
        fs::rename(
            temp.path().join(format!("storm-{index}")),
            temp.path().join(format!("storm-{}", index + 1)),
        )
        .map_err(|error| error.to_string())?;
    }
    let outside = tempfile::tempdir().map_err(|error| error.to_string())?;
    fs::write(temp.path().join("departing"), b"departing").map_err(|error| error.to_string())?;
    fs::rename(
        temp.path().join("departing"),
        outside.path().join("departed"),
    )
    .map_err(|error| error.to_string())?;
    thread::sleep(COOKIE_EXPIRY + Duration::from_millis(50));
    let events = events_through(&observer)?;
    if !has_event(&events, "storm-0", EvidenceFlags::RENAME_FROM)
        || !has_event(&events, "storm-128", EvidenceFlags::RENAME_TO)
    {
        return Err("rename storm lost an endpoint".into());
    }
    if !has_event(
        &events,
        "departing",
        EvidenceFlags::PROVIDER_COMPLETE_PREFIX,
    ) {
        return Err("expired rename cookie did not conservatively dirty its prefix".into());
    }
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn run_delayed_backlog() -> std::result::Result<(), String> {
    let (temp, observer) = fixture()?;
    for index in 0..512 {
        fs::write(temp.path().join(format!("backlog-{index}")), b"queued")
            .map_err(|error| error.to_string())?;
    }
    thread::sleep(Duration::from_millis(20));
    let events = events_through(&observer)?;
    if !has_event(&events, "backlog-511", EvidenceFlags::CREATE) {
        return Err("durable fence returned before delayed backlog".into());
    }
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn run_fence_ordering() -> std::result::Result<(), String> {
    let (temp, observer) = fixture()?;
    fs::write(temp.path().join("before"), b"before").map_err(|error| error.to_string())?;
    let fence = observer
        .begin_observation(&expected_for(&observer, 2))
        .map_err(|error| error.to_string())?;
    let state = observer.shared.lock();
    let sentinel = state
        .events
        .iter()
        .filter(|item| {
            item.event
                .path
                .as_str()
                .starts_with(".trail-observer-fence-")
        })
        .collect::<Vec<_>>();
    let create = sentinel
        .iter()
        .find(|item| item.event.flags.0 & EvidenceFlags::CREATE.0 != 0)
        .ok_or_else(|| "fence create was not durably observed".to_string())?;
    let delete = sentinel
        .iter()
        .find(|item| item.event.flags.0 & EvidenceFlags::DELETE.0 != 0)
        .ok_or_else(|| "fence delete was not durably observed".to_string())?;
    if create.event.sequence >= delete.event.sequence
        || create.cut.durable_end_offset >= delete.cut.durable_end_offset
        || fence.sequence != delete.event.sequence
    {
        return Err("sentinel durable create/delete ordering is invalid".into());
    }
    drop(state);
    if fs::read_dir(temp.path())
        .map_err(|error| error.to_string())?
        .any(|entry| {
            entry
                .ok()
                .and_then(|entry| entry.file_name().to_str().map(str::to_owned))
                .is_some_and(|name| name.starts_with(".trail-observer-fence-"))
        })
    {
        return Err("sentinel remained after the delete fence".into());
    }
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn run_fault_revocation_matrix() -> std::result::Result<(), String> {
    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let observer = LinuxInotifyObserver::start(
        temp.path(),
        Box::new(SlowDurability {
            inner: memory_durability(None),
        }),
    )
    .map_err(|error| error.to_string())?;
    for index in 0..6_000 {
        fs::write(temp.path().join(format!("overflow-{index}")), b"overflow")
            .map_err(|error| error.to_string())?;
    }
    expect_revoked(&observer, "overflow")?;

    let (_temp, observer) = fixture()?;
    if classify_raw_authority_event(&observer.shared, EventMask::CREATE, false).is_ok() {
        return Err("raw unknown watch descriptor passed the authority classifier".into());
    }
    expect_revoked(&observer, "inotify_unknown_watch_descriptor")?;

    let (temp, observer) = fixture()?;
    observer.test_fail_next_watch_add();
    fs::create_dir(temp.path().join("watch-add-fails")).map_err(|error| error.to_string())?;
    expect_revoked(&observer, "inotify_watch_add_failure")?;

    let (temp, observer) = fixture()?;
    fs::create_dir(temp.path().join("ignored")).map_err(|error| error.to_string())?;
    observer
        .begin_observation(&expected_for(&observer, 3))
        .map_err(|error| error.to_string())?;
    fs::remove_dir(temp.path().join("ignored")).map_err(|error| error.to_string())?;
    expect_revoked(&observer, "inotify_watch_ignored")?;

    use std::os::unix::ffi::OsStringExt;
    let (temp, observer) = fixture()?;
    let bad = OsString::from_vec(vec![b'b', b'a', b'd', 0xff]);
    fs::write(temp.path().join(bad), b"ambiguous").map_err(|error| error.to_string())?;
    expect_revoked(&observer, "inotify_path_decode_ambiguity")?;

    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    let observer = LinuxInotifyObserver::start(temp.path(), Box::new(memory_durability(Some(0))))
        .map_err(|error| error.to_string())?;
    fs::write(temp.path().join("durability-fails"), b"fail").map_err(|error| error.to_string())?;
    expect_revoked(&observer, "inotify_durability_failure")?;
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn run_owner_death_and_root_replacement() -> std::result::Result<(), String> {
    use std::io::BufRead;
    use std::process::{Command, Stdio};

    let native = NativeFixture::new(|_| Ok(())).map_err(|error| error.to_string())?;
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let mut child = Command::new(executable)
        .arg("linux_observer_process_owner_child")
        .arg("--exact")
        .arg("--nocapture")
        .env("TRAIL_LINUX_OBSERVER_CHILD_ROOT", &native.db.workspace_root)
        .env("TRAIL_LINUX_OBSERVER_CHILD_SQLITE", "1")
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|error| error.to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "observer owner child stdout was unavailable".to_string())?;
    let mut ready = false;
    for line in std::io::BufReader::new(stdout).lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.contains("TRAIL_LINUX_OBSERVER_OWNER_READY") {
            ready = true;
            break;
        }
    }
    if !ready {
        let _ = child.kill();
        return Err("observer owner child exited before readiness".into());
    }
    child.kill().map_err(|error| error.to_string())?;
    let status = child.wait().map_err(|error| error.to_string())?;
    if status.success() {
        return Err("observer owner child was not killed".into());
    }
    let persisted_owner: (i64, String, String) = native
        .db
        .conn
        .query_row(
            "SELECT epoch,owner_token,lease_state FROM changed_path_observer_owners
             WHERE scope_id=?1",
            [native.expected.scope_id.to_text()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(|error| error.to_string())?;
    if persisted_owner.0 != 1 || persisted_owner.1.is_empty() || persisted_owner.2 != "active" {
        return Err("killed observer owner was not persisted as the active epoch owner".into());
    }
    let replacement_owner = [0xa5; 32];
    if SegmentWriter::acquire(
        &native.db.sqlite_path,
        &native.segment_directory,
        native.expected.scope_id,
        native.expected.epoch,
        replacement_owner,
        &hex::encode(&native.expected.provider_identity),
        Vec::new(),
        Duration::from_secs(3_600),
    )
    .is_ok()
    {
        return Err("same-epoch owner replacement succeeded after SIGKILL".into());
    }
    native
        .db
        .conn
        .execute(
            "UPDATE changed_path_scopes
             SET epoch=2,trust_state='reconciling',trust_reason='authoritative_epoch_advance',
                 continuity_generation=continuity_generation+1,observer_owner_token=NULL,
                 durable_offset=0,folded_offset=0
             WHERE scope_id=?1 AND epoch=1",
            [native.expected.scope_id.to_text()],
        )
        .map_err(|error| error.to_string())?;
    let writer = SegmentWriter::acquire(
        &native.db.sqlite_path,
        &native.segment_directory,
        native.expected.scope_id,
        2,
        replacement_owner,
        &hex::encode(&native.expected.provider_identity),
        Vec::new(),
        Duration::from_secs(3_600),
    )
    .map_err(|error| error.to_string())?;
    let durability = SegmentWriterDurability::new(
        writer,
        native.expected.provider_identity.clone(),
        vec![0x5a; 24],
    )
    .map_err(|error| error.to_string())?;
    let replacement = LinuxInotifyObserver::start(&native.db.workspace_root, Box::new(durability))
        .map_err(|error| error.to_string())?;
    let mut advanced = native.expected.clone();
    advanced.epoch = 2;
    replacement
        .begin_observation(&advanced)
        .map_err(|error| error.to_string())?;

    let (temp, observer) = fixture()?;
    let root = temp.path().to_path_buf();
    let displaced = root.with_extension("displaced");
    fs::rename(&root, &displaced).map_err(|error| error.to_string())?;
    fs::create_dir(&root).map_err(|error| error.to_string())?;
    expect_revoked(&observer, "inotify_root")?;
    fs::remove_dir_all(&root).map_err(|error| error.to_string())?;
    fs::rename(&displaced, &root).map_err(|error| error.to_string())?;
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn run_complete_prefix_publication_races() -> std::result::Result<(), String> {
    fn run() -> Result<()> {
        let fixture = NativeFixture::new(|_| Ok(()))?;
        let outside = tempfile::tempdir()?;
        fs::create_dir_all(outside.path().join("populated/deep"))?;
        fs::write(outside.path().join("populated/one"), b"one")?;
        fs::write(outside.path().join("populated/deep/two"), b"two")?;
        let source = outside.path().join("populated");
        let destination = fixture.db.workspace_root.join("incoming");
        let (start_move, receive_move) = mpsc::channel();
        let (moved, receive_moved) = mpsc::channel();
        let mover = thread::spawn(move || -> Result<()> {
            receive_move
                .recv()
                .map_err(|_| Error::InvalidInput("prefix race start signal lost".into()))?;
            fs::rename(source, destination)?;
            moved
                .send(())
                .map_err(|_| Error::InvalidInput("prefix race completion signal lost".into()))?;
            Ok(())
        });
        install_initial_scan_hook(fixture.expected.scope_id, move || {
            start_move
                .send(())
                .map_err(|_| Error::InvalidInput("prefix race mover unavailable".into()))?;
            receive_moved
                .recv()
                .map_err(|_| Error::InvalidInput("prefix race move acknowledgement lost".into()))
        });
        let observer = fixture.observer()?;
        let report = reconcile_full(
            &fixture.db,
            &fixture.db.changed_path_ledger(),
            &observer,
            &fixture.expected,
            &fixture.policy,
            "linux_complete_prefix_move_in",
        )?;
        mover
            .join()
            .map_err(|_| Error::InvalidInput("prefix race mover panicked".into()))??;
        if !report.published {
            return Err(Error::Corrupt(
                "move-in prefix reconciliation did not publish".into(),
            ));
        }
        let paths = fixture.published_paths()?;
        if !paths.contains(&"incoming/one".to_string())
            || !paths.contains(&"incoming/deep/two".to_string())
        {
            return Err(Error::Corrupt(format!(
                "move-in prefix reconciliation omitted descendants: {paths:?}"
            )));
        }

        let fixture = NativeFixture::new(|root| {
            fs::create_dir_all(root.join("old/deep"))?;
            fs::write(root.join("old/one"), b"one")?;
            fs::write(root.join("old/deep/two"), b"two")?;
            Ok(())
        })?;
        let old = fixture.db.workspace_root.join("old");
        let new = fixture.db.workspace_root.join("new");
        let (start_move, receive_move) = mpsc::channel();
        let (moved, receive_moved) = mpsc::channel();
        let mover = thread::spawn(move || -> Result<()> {
            receive_move
                .recv()
                .map_err(|_| Error::InvalidInput("rename race start signal lost".into()))?;
            fs::rename(old, new)?;
            moved
                .send(())
                .map_err(|_| Error::InvalidInput("rename race completion signal lost".into()))?;
            Ok(())
        });
        install_initial_scan_hook(fixture.expected.scope_id, move || {
            start_move
                .send(())
                .map_err(|_| Error::InvalidInput("rename race mover unavailable".into()))?;
            receive_moved
                .recv()
                .map_err(|_| Error::InvalidInput("rename race acknowledgement lost".into()))
        });
        let observer = fixture.observer()?;
        let report = reconcile_full(
            &fixture.db,
            &fixture.db.changed_path_ledger(),
            &observer,
            &fixture.expected,
            &fixture.policy,
            "linux_complete_prefix_directory_rename",
        )?;
        mover
            .join()
            .map_err(|_| Error::InvalidInput("rename race mover panicked".into()))??;
        if !report.published {
            return Err(Error::Corrupt(
                "directory rename reconciliation did not publish".into(),
            ));
        }
        let paths = fixture.published_paths()?;
        for required in ["old/one", "old/deep/two", "new/one", "new/deep/two"] {
            if !paths.contains(&required.to_string()) {
                return Err(Error::Corrupt(format!(
                    "directory rename prefix reconciliation omitted {required}: {paths:?}"
                )));
            }
        }
        Ok(())
    }
    run().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_authenticated_fence_rejections() -> std::result::Result<(), String> {
    fn must_reject(mutate: impl FnOnce(&mut ObserverFence)) -> std::result::Result<(), String> {
        let (_temp, observer) = fixture()?;
        let expected = expected_for(&observer, 0x31);
        let mut start = observer
            .begin_observation(&expected)
            .map_err(|error| error.to_string())?;
        mutate(&mut start);
        if observer.end_fence(&expected, &start).is_ok() {
            return Err("forged issued fence was accepted".into());
        }
        Ok(())
    }
    must_reject(|fence| fence.sequence = fence.sequence.saturating_add(1))?;
    must_reject(|fence| fence.durable_offset = fence.durable_offset.saturating_add(1))?;
    must_reject(|fence| fence.nonce[0] ^= 0xff)?;

    let (_temp, observer) = fixture()?;
    let expected = expected_for(&observer, 0x32);
    let other = expected_for(&observer, 0x33);
    let start = observer
        .begin_observation(&expected)
        .map_err(|error| error.to_string())?;
    if observer.end_fence(&other, &start).is_ok() {
        return Err("cross-scope issued fence was accepted".into());
    }

    let (_temp, observer) = fixture()?;
    let expected = expected_for(&observer, 0x34);
    let start = observer
        .begin_observation(&expected)
        .map_err(|error| error.to_string())?;
    let end = observer
        .end_fence(&expected, &start)
        .map_err(|error| error.to_string())?;
    observer
        .drain_through(
            &expected,
            &observer.root_identity,
            &start,
            &end,
            &mut |_| Ok(()),
        )
        .map_err(|error| error.to_string())?;
    if observer
        .drain_through(
            &expected,
            &observer.root_identity,
            &start,
            &end,
            &mut |_| Ok(()),
        )
        .is_ok()
    {
        return Err("consumed issued fence interval was replayed".into());
    }

    let native = NativeFixture::new(|_| Ok(())).map_err(|error| error.to_string())?;
    let observer = native.observer().map_err(|error| error.to_string())?;
    let start = observer
        .begin_observation(&native.expected)
        .map_err(|error| error.to_string())?;
    native
        .db
        .conn
        .execute(
            "UPDATE changed_path_observer_owners SET lease_state='revoked'
             WHERE scope_id=?1",
            [native.expected.scope_id.to_text()],
        )
        .map_err(|error| error.to_string())?;
    if observer.end_fence(&native.expected, &start).is_ok() {
        return Err("persisted observer owner replacement retained fence authority".into());
    }

    let native = NativeFixture::new(|_| Ok(())).map_err(|error| error.to_string())?;
    let observer = native.observer().map_err(|error| error.to_string())?;
    let start = observer
        .begin_observation(&native.expected)
        .map_err(|error| error.to_string())?;
    native
        .db
        .conn
        .execute(
            "UPDATE changed_path_observer_segments SET owner_token='replacement-owner'
             WHERE scope_id=?1",
            [native.expected.scope_id.to_text()],
        )
        .map_err(|error| error.to_string())?;
    if observer.end_fence(&native.expected, &start).is_ok() {
        return Err("persisted observer segment replacement retained fence authority".into());
    }
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn run_segment_writer_reconcile_publication() -> std::result::Result<(), String> {
    fn run() -> Result<()> {
        let fixture = NativeFixture::new(|root| {
            fs::write(root.join("tracked"), b"before")?;
            Ok(())
        })?;
        let observer = fixture.observer()?;
        fs::write(fixture.db.workspace_root.join("tracked"), b"after")?;
        let report = reconcile_full(
            &fixture.db,
            &fixture.db.changed_path_ledger(),
            &observer,
            &fixture.expected,
            &fixture.policy,
            "linux_native_segment_writer",
        )?;
        if !report.published || !fixture.published_paths()?.contains(&"tracked".into()) {
            return Err(Error::Corrupt(
                "native SegmentWriter reconciliation did not publish".into(),
            ));
        }
        let (scope_folded, segment_folded, owner_matches): (i64, i64, bool) =
            fixture.db.conn.query_row(
                "SELECT scope.folded_offset,segment.folded_end_offset,
                    owner.owner_token=scope.observer_owner_token
                       AND owner.provider_identity=scope.provider_identity
                       AND owner.fence_nonce IS NOT NULL
             FROM changed_path_scopes scope
             JOIN changed_path_observer_owners owner ON owner.scope_id=scope.scope_id
             JOIN changed_path_observer_segments segment
               ON segment.scope_id=scope.scope_id AND segment.owner_token=owner.owner_token
             WHERE scope.scope_id=?1",
                [fixture.expected.scope_id.to_text()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )?;
        if scope_folded != i64::try_from(report.end_durable_offset).unwrap_or(-1)
            || segment_folded != scope_folded
            || !owner_matches
        {
            return Err(Error::Corrupt(
                "native owner/fence/fold binding was not exact".into(),
            ));
        }

        let fixture = NativeFixture::new(|root| {
            fs::write(root.join("rollback"), b"before")?;
            fs::write(root.join("rollback-two"), b"before")?;
            Ok(())
        })?;
        let observer = fixture.observer()?;
        fs::write(fixture.db.workspace_root.join("rollback"), b"after")?;
        fs::write(fixture.db.workspace_root.join("rollback-two"), b"after")?;
        let ledger = fixture.db.changed_path_ledger();
        let mut attempt = begin_reconciliation(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            ReconcileMode::Full,
            "linux_atomic_fold_rollback",
        )?;
        attempt.observe(&fixture.db, &ledger, &observer, &fixture.policy)?;
        let before_fold: i64 = fixture.db.conn.query_row(
            "SELECT folded_end_offset FROM changed_path_observer_segments
             WHERE scope_id=?1",
            [fixture.expected.scope_id.to_text()],
            |row| row.get(0),
        )?;
        if before_fold != 0 {
            return Err(Error::Corrupt(
                "drain persisted folded offset before publication".into(),
            ));
        }
        fixture.db.conn.execute(
            "UPDATE changed_path_scopes SET max_candidate_rows=1 WHERE scope_id=?1",
            [fixture.expected.scope_id.to_text()],
        )?;
        if attempt
            .publish(&fixture.db, &ledger, &fixture.policy)
            .is_ok()
        {
            return Err(Error::Corrupt(
                "candidate-cap failure unexpectedly published".into(),
            ));
        }
        let after_fold: i64 = fixture.db.conn.query_row(
            "SELECT folded_end_offset FROM changed_path_observer_segments
             WHERE scope_id=?1",
            [fixture.expected.scope_id.to_text()],
            |row| row.get(0),
        )?;
        if after_fold != 0 {
            return Err(Error::Corrupt(
                "failed publication did not roll back folded offset".into(),
            ));
        }
        Ok(())
    }
    run().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_raw_decoder_faults() -> std::result::Result<(), String> {
    let (_temp, observer) = fixture()?;
    if classify_raw_authority_event(&observer.shared, EventMask::Q_OVERFLOW, false).is_ok() {
        return Err("raw IN_Q_OVERFLOW passed the authority classifier".into());
    }
    expect_revoked(&observer, "inotify_queue_overflow")?;
    let (_temp, observer) = fixture()?;
    if classify_raw_authority_event(&observer.shared, EventMask::CREATE, false).is_ok() {
        return Err("raw unknown watch descriptor passed the authority classifier".into());
    }
    expect_revoked(&observer, "inotify_unknown_watch_descriptor")
}

#[cfg(debug_assertions)]
pub(crate) fn run_process_owner_child(root: &str) -> std::result::Result<(), String> {
    let _database;
    let _observer = if std::env::var_os("TRAIL_LINUX_OBSERVER_CHILD_SQLITE").is_some() {
        let database = Trail::open(Path::new(root)).map_err(|error| error.to_string())?;
        let provider_identity = b"linux-inotify-native-v1".to_vec();
        let mut owner = [0_u8; 32];
        let mut fence = [0_u8; 24];
        getrandom::getrandom(&mut owner).map_err(|error| error.to_string())?;
        getrandom::getrandom(&mut fence).map_err(|error| error.to_string())?;
        let writer = SegmentWriter::acquire(
            &database.sqlite_path,
            &database.db_dir.join("change-observer-segments"),
            ScopeId([0x91; 32]),
            1,
            owner,
            &hex::encode(&provider_identity),
            Vec::new(),
            Duration::from_secs(3_600),
        )
        .map_err(|error| error.to_string())?;
        let durability = SegmentWriterDurability::new(writer, provider_identity, fence.to_vec())
            .map_err(|error| error.to_string())?;
        let observer = LinuxInotifyObserver::start(Path::new(root), Box::new(durability))
            .map_err(|error| error.to_string())?;
        _database = Some(database);
        observer
    } else {
        _database = None;
        LinuxInotifyObserver::start(Path::new(root), Box::new(memory_durability(None)))
            .map_err(|error| error.to_string())?
    };
    println!("TRAIL_LINUX_OBSERVER_OWNER_READY");
    std::io::stdout()
        .flush()
        .map_err(|error| error.to_string())?;
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}
