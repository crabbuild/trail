//! Qualified macOS FSEvents observer.
//!
//! The callback is deliberately small: it validates the native flags and
//! path, then performs one bounded `try_send`.  Segment I/O and SQLite lease
//! validation belong exclusively to the durability worker.

use std::collections::HashMap;
use std::ffi::CStr;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::raw::c_void;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Component, Path, PathBuf};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use fsevent_sys as fs_events;
use rustix::fs::{fsync, openat, unlinkat, AtFlags, Mode, OFlags};
use serde::{Deserialize, Serialize};

use super::{ObserverFence, ObserverLease, QualifiedObserver};
use crate::db::change_ledger::reconcile::{ObserverEvent, ObserverQualification};
use crate::db::change_ledger::secure_fs::SecureDirectory;
#[cfg(debug_assertions)]
use crate::db::change_ledger::{
    BaselineIdentity, FilesystemIdentity, PolicyIdentity, ProviderIdentity, ScopeId, ScopeIdentity,
    ScopeKind,
};
use crate::db::change_ledger::{
    DurableCut, EvidenceFlags, EvidenceSource, ExpectedScope, LedgerPath, ObserverRecord,
    ObserverWriterBinding, ProviderCapabilities, SegmentWriter,
};
use crate::error::{Error, Result};
#[cfg(debug_assertions)]
use crate::{InitImportMode, Trail};

const MAX_PENDING_RECORDS: usize = 8_192;
const MAX_RETAINED_EVENTS: usize = 65_536;
const FENCE_TIMEOUT: Duration = Duration::from_secs(10);

const CAPABILITY_VERSION: u16 = 1;
const STREAM_FLAGS: u32 = fs_events::kFSEventStreamCreateFlagFileEvents
    | fs_events::kFSEventStreamCreateFlagNoDefer
    | fs_events::kFSEventStreamCreateFlagWatchRoot;
const GAP_FLAGS: u32 = fs_events::kFSEventStreamEventFlagMustScanSubDirs
    | fs_events::kFSEventStreamEventFlagUserDropped
    | fs_events::kFSEventStreamEventFlagKernelDropped
    | fs_events::kFSEventStreamEventFlagEventIdsWrapped
    | fs_events::kFSEventStreamEventFlagRootChanged
    | fs_events::kFSEventStreamEventFlagUnmount;

pub(crate) trait MacObserverDurability: Send {
    fn binding(&self) -> ObserverWriterBinding;
    fn append_and_flush(&mut self, record: ObserverRecord) -> Result<DurableCut>;
}

pub(crate) struct MacSegmentWriterDurability {
    writer: SegmentWriter,
    binding: ObserverWriterBinding,
}

impl MacSegmentWriterDurability {
    pub(crate) fn new(
        mut writer: SegmentWriter,
        provider_identity: Vec<u8>,
        fence_nonce: Vec<u8>,
    ) -> Result<Self> {
        let binding = writer.bind_native_observer(provider_identity, fence_nonce)?;
        Ok(Self { writer, binding })
    }
}

impl MacObserverDurability for MacSegmentWriterDurability {
    fn binding(&self) -> ObserverWriterBinding {
        self.binding.clone()
    }

    fn append_and_flush(&mut self, record: ObserverRecord) -> Result<DurableCut> {
        self.writer.append(&[record])?;
        self.writer.flush_durable()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct MacOsProviderCursor {
    version: u16,
    event_id: u64,
    device: u64,
    root_identity: Vec<u8>,
    stream_identity: Vec<u8>,
    provider_identity: Vec<u8>,
    stream_flags: u32,
    capabilities: ProviderCapabilities,
}

impl MacOsProviderCursor {
    pub(crate) fn decode(bytes: &[u8]) -> Result<Self> {
        crate::error::from_cbor(bytes)
    }

    fn encode(&self) -> Result<Vec<u8>> {
        crate::error::cbor(self)
    }

    fn validate_resume(
        &self,
        root_identity: &[u8],
        device: u64,
        provider_identity: &[u8],
    ) -> Result<()> {
        if self.version != CAPABILITY_VERSION
            || self.event_id == fs_events::kFSEventStreamEventIdSinceNow
            || self.device != device
            || self.root_identity != root_identity
            || self.stream_identity.len() < 16
            || self.provider_identity != provider_identity
            || self.stream_flags != STREAM_FLAGS
            || self.capabilities != native_capabilities()
        {
            return Err(reconcile_error(
                "fsevents_resume_identity_or_capability_mismatch",
            ));
        }
        let current = unsafe { fs_events::FSEventsGetCurrentEventId() };
        if self.event_id > current {
            return Err(reconcile_error(
                "fsevents_resume_cursor_is_from_replaced_history",
            ));
        }
        Ok(())
    }
}

#[derive(Clone)]
struct DurableEvent {
    event: ObserverEvent,
    provider_event_id: u64,
    cut: DurableCut,
    internal_fence: bool,
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
    owner_fence_nonce: Vec<u8>,
    provider_event_id: u64,
    durable_cut: DurableCut,
    kind: IssuedFenceKind,
}

struct State {
    active: bool,
    revoked: Option<String>,
    history_required: bool,
    history_done: bool,
    events: Vec<DurableEvent>,
    next_sequence: u64,
    last_provider_event_id: u64,
    last_cursor: Option<MacOsProviderCursor>,
    issued_fences: HashMap<Vec<u8>, IssuedFence>,
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

struct CallbackContext {
    root_path: PathBuf,
    policy_dependencies: Vec<LedgerPath>,
    records: SyncSender<DurabilityCommand>,
    shared: Arc<Shared>,
}

enum DurabilityCommand {
    Record {
        path: LedgerPath,
        flags: EvidenceFlags,
        provider_event_id: u64,
    },
    Fence {
        provider_event_id: u64,
        nonce: Vec<u8>,
        response: SyncSender<Result<(ObserverFence, DurableCut)>>,
    },
    #[cfg(debug_assertions)]
    StopForTest,
    Shutdown,
}

#[derive(Clone, Copy)]
struct StreamHandle {
    stream: usize,
    run_loop: usize,
}

pub(crate) struct MacOsFseventsObserver {
    root_path: PathBuf,
    root: File,
    root_identity: Vec<u8>,
    fence_directory: SecureDirectory,
    fence_directory_identity: (u64, u64),
    device: u64,
    provider_identity: Vec<u8>,
    owner_token: String,
    owner_fence_nonce: Vec<u8>,
    stream_identity: Vec<u8>,
    shared: Arc<Shared>,
    commands: SyncSender<DurabilityCommand>,
    stream: StreamHandle,
    workers: Mutex<Vec<JoinHandle<()>>>,
    #[cfg(debug_assertions)]
    next_test_fence_nonce: Mutex<Option<Vec<u8>>>,
    #[cfg(debug_assertions)]
    fail_next_fence_sync: Mutex<bool>,
}

impl MacOsFseventsObserver {
    pub(crate) fn start(
        root_path: &Path,
        durability: Box<dyn MacObserverDurability>,
        resume: Option<MacOsProviderCursor>,
        policy_dependencies: &[PathBuf],
    ) -> Result<Self> {
        let requested_root = root_path.to_path_buf();
        let root_path = root_path.canonicalize()?;
        let root = open_root_no_follow(&root_path)?;
        let root_identity = root_identity(&root)?;
        let policy_dependencies =
            normalize_policy_dependencies(&root_path, &requested_root, policy_dependencies)?;
        let secure_root = SecureDirectory::open_absolute(&root_path)?;
        let trail_directory = secure_root
            .open_dir(".trail")
            .map_err(|_| reconcile_error("fsevents_workspace_storage_absent_or_unsafe"))?;
        let fence_directory = match trail_directory.open_private_dir("observer-fences") {
            Ok(directory) => directory,
            Err(_) => trail_directory.create_private_dir("observer-fences")?,
        };
        let fence_directory_identity = fence_directory.identity()?;
        let device = root.metadata()?.dev();
        let binding = durability.binding();
        if binding.owner_token.is_empty()
            || binding.provider_id != hex::encode(&binding.provider_identity)
            || binding.provider_identity.is_empty()
            || binding.fence_nonce.len() < 16
        {
            return Err(Error::InvalidInput(
                "native macOS observer durability binding is incomplete".into(),
            ));
        }
        if let Some(cursor) = &resume {
            cursor.validate_resume(&root_identity, device, &binding.provider_identity)?;
        }
        let mut stream_identity = resume
            .as_ref()
            .map(|cursor| cursor.stream_identity.clone())
            .unwrap_or_else(|| vec![0_u8; 24]);
        if resume.is_none() {
            getrandom::getrandom(&mut stream_identity).map_err(|error| {
                Error::InvalidInput(format!("FSEvents stream identity entropy failed: {error}"))
            })?;
        }
        let since_when = resume
            .as_ref()
            .map(|cursor| cursor.event_id)
            .unwrap_or(fs_events::kFSEventStreamEventIdSinceNow);
        let shared = Arc::new(Shared {
            state: Mutex::new(State {
                active: true,
                revoked: None,
                history_required: resume.is_some(),
                history_done: resume.is_none(),
                events: Vec::new(),
                next_sequence: 1,
                last_provider_event_id: resume.as_ref().map_or(0, |cursor| cursor.event_id),
                last_cursor: resume.clone(),
                issued_fences: HashMap::new(),
            }),
            changed: Condvar::new(),
            shutdown: AtomicBool::new(false),
        });
        let (commands, records) = mpsc::sync_channel(MAX_PENDING_RECORDS);
        let durability_shared = Arc::clone(&shared);
        let cursor_template = MacOsProviderCursor {
            version: CAPABILITY_VERSION,
            event_id: 0,
            device,
            root_identity: root_identity.clone(),
            stream_identity: stream_identity.clone(),
            provider_identity: binding.provider_identity.clone(),
            stream_flags: STREAM_FLAGS,
            capabilities: native_capabilities(),
        };
        let durability_worker = thread::Builder::new()
            .name("trail-macos-observer-durability".into())
            .spawn(move || {
                run_durability_worker(records, durability, durability_shared, cursor_template)
            })?;

        let callback = Box::new(CallbackContext {
            root_path: root_path.clone(),
            policy_dependencies,
            records: commands.clone(),
            shared: Arc::clone(&shared),
        });
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let stream_shared = Arc::clone(&shared);
        let stream_root = root_path.clone();
        let stream_worker = match thread::Builder::new()
            .name("trail-macos-fsevents".into())
            .spawn(move || run_stream(stream_root, since_when, callback, ready_tx, stream_shared))
        {
            Ok(worker) => worker,
            Err(error) => {
                shared.shutdown.store(true, Ordering::Release);
                let _ = commands.send(DurabilityCommand::Shutdown);
                let _ = durability_worker.join();
                return Err(Error::Io(error));
            }
        };
        let stream = match ready_rx.recv_timeout(FENCE_TIMEOUT) {
            Ok(Ok(stream)) => stream,
            Ok(Err(error)) => {
                shared.revoke("fsevents_stream_start_failure");
                shared.shutdown.store(true, Ordering::Release);
                let _ = commands.send(DurabilityCommand::Shutdown);
                let _ = stream_worker.join();
                let _ = durability_worker.join();
                return Err(error);
            }
            Err(_) => {
                shared.revoke("fsevents_stream_start_timeout");
                shared.shutdown.store(true, Ordering::Release);
                let _ = commands.send(DurabilityCommand::Shutdown);
                let _ = stream_worker.join();
                let _ = durability_worker.join();
                return Err(reconcile_error("fsevents_stream_start_timeout"));
            }
        };
        let observer = Self {
            root_path,
            root,
            root_identity,
            fence_directory,
            fence_directory_identity,
            device,
            provider_identity: binding.provider_identity,
            owner_token: binding.owner_token,
            owner_fence_nonce: binding.fence_nonce,
            stream_identity,
            shared,
            commands,
            stream,
            workers: Mutex::new(vec![stream_worker, durability_worker]),
            #[cfg(debug_assertions)]
            next_test_fence_nonce: Mutex::new(None),
            #[cfg(debug_assertions)]
            fail_next_fence_sync: Mutex::new(false),
        };
        observer.wait_for_history()?;
        observer.root_identity()?;
        Ok(observer)
    }

    pub(crate) fn capabilities(&self) -> ProviderCapabilities {
        native_capabilities()
    }

    pub(crate) fn lease(&self) -> Result<ObserverLease> {
        Ok(ObserverLease {
            owner_token: self.owner_token.clone(),
            root_identity: self.root_identity()?,
            capabilities: self.capabilities(),
        })
    }

    pub(crate) fn resume_cursor(&self) -> Result<Option<MacOsProviderCursor>> {
        self.ensure_available()?;
        Ok(self.shared.lock().last_cursor.clone())
    }

    fn ensure_available(&self) -> Result<()> {
        let state = self.shared.lock();
        if let Some(reason) = &state.revoked {
            return Err(reconcile_error(reason));
        }
        if !state.active {
            return Err(reconcile_error("fsevents_observer_unavailable"));
        }
        if state.history_required && !state.history_done {
            return Err(reconcile_error("fsevents_history_not_complete"));
        }
        Ok(())
    }

    fn wait_for_history(&self) -> Result<()> {
        let deadline = Instant::now() + FENCE_TIMEOUT;
        let mut state = self.shared.lock();
        while state.history_required && !state.history_done && state.revoked.is_none() {
            let now = Instant::now();
            if now >= deadline {
                drop(state);
                self.shared.revoke("fsevents_history_done_timeout");
                return Err(reconcile_error("fsevents_history_done_timeout"));
            }
            let waited = self
                .shared
                .changed
                .wait_timeout(state, deadline.saturating_duration_since(now))
                .unwrap_or_else(|poison| poison.into_inner());
            state = waited.0;
        }
        if let Some(reason) = &state.revoked {
            return Err(reconcile_error(reason));
        }
        Ok(())
    }

    pub(crate) fn root_identity(&self) -> Result<Vec<u8>> {
        self.ensure_available()?;
        if root_identity(&self.root)? != self.root_identity
            || root_identity(&open_root_no_follow(&self.root_path)?)? != self.root_identity
        {
            self.shared.revoke("fsevents_root_replaced");
            return Err(reconcile_error("fsevents_root_replaced"));
        }
        Ok(self.root_identity.clone())
    }

    fn flush_fence(
        &self,
        expected: &ExpectedScope,
        kind: IssuedFenceKind,
    ) -> Result<ObserverFence> {
        self.ensure_available()?;
        self.root_identity()?;
        if expected.provider_identity != self.provider_identity
            || expected.filesystem_identity != self.root_identity
        {
            self.shared.revoke("fsevents_expected_identity_mismatch");
            return Err(reconcile_error("fsevents_expected_identity_mismatch"));
        }
        #[cfg(debug_assertions)]
        let injected_nonce = self
            .next_test_fence_nonce
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take();
        #[cfg(not(debug_assertions))]
        let injected_nonce: Option<Vec<u8>> = None;
        let mut nonce = injected_nonce.unwrap_or_else(|| vec![0_u8; 24]);
        if nonce.iter().all(|byte| *byte == 0) {
            getrandom::getrandom(&mut nonce).map_err(|error| {
                Error::InvalidInput(format!("FSEvents fence entropy failed: {error}"))
            })?;
        }
        if nonce.len() < 16 {
            self.shared.revoke("fsevents_fence_nonce_invalid");
            return Err(reconcile_error("fsevents_fence_nonce_invalid"));
        }
        self.fence_directory
            .verify_identity(self.fence_directory_identity)
            .map_err(|_| {
                self.shared.revoke("fsevents_fence_directory_replaced");
                reconcile_error("fsevents_fence_directory_replaced")
            })?;
        let sentinel_name = hex::encode(&nonce);
        let sentinel_path = LedgerPath::parse(&format!(".trail/observer-fences/{sentinel_name}"))?;
        let fd = match openat(
            self.fence_directory.file(),
            Path::new(&sentinel_name),
            OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::RUSR | Mode::WUSR,
        ) {
            Ok(fd) => fd,
            Err(_) => {
                self.shared
                    .revoke("fsevents_fence_collision_or_create_failure");
                return Err(reconcile_error(
                    "fsevents_fence_collision_or_create_failure",
                ));
            }
        };
        let mut sentinel = File::from(fd);
        if sentinel.write_all(hex::encode(&nonce).as_bytes()).is_err() {
            return Err(self.fail_fence(&sentinel_name, "fsevents_fence_write_failure"));
        }
        #[cfg(debug_assertions)]
        let injected_sync_failure = {
            let mut fail = self
                .fail_next_fence_sync
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            let injected = *fail;
            *fail = false;
            injected
        };
        #[cfg(not(debug_assertions))]
        let injected_sync_failure = false;
        if injected_sync_failure || sentinel.sync_all().is_err() {
            return Err(self.fail_fence(&sentinel_name, "fsevents_fence_file_sync_failure"));
        }
        if fsync(self.fence_directory.file()).is_err() {
            return Err(self.fail_fence(&sentinel_name, "fsevents_fence_parent_sync_failure"));
        }
        // FlushSync alone cannot order a write which fseventsd has not yet
        // ingested.  The private create is the first journal barrier; its
        // durable callback proves every earlier journal entry was ingested.
        unsafe {
            fs_events::FSEventStreamFlushSync(self.stream.stream as fs_events::FSEventStreamRef);
        }
        if let Err(error) = self.wait_for_sentinel(&sentinel_path, EvidenceFlags::CREATE) {
            let _ = unlinkat(
                self.fence_directory.file(),
                Path::new(&sentinel_name),
                AtFlags::empty(),
            );
            let _ = fsync(self.fence_directory.file());
            return Err(error);
        }
        if unlinkat(
            self.fence_directory.file(),
            Path::new(&sentinel_name),
            AtFlags::empty(),
        )
        .is_err()
        {
            self.shared.revoke("fsevents_fence_cleanup_failure");
            return Err(reconcile_error("fsevents_fence_cleanup_failure"));
        }
        if fsync(self.fence_directory.file()).is_err() {
            self.shared
                .revoke("fsevents_fence_post_unlink_parent_sync_failure");
            return Err(reconcile_error(
                "fsevents_fence_post_unlink_parent_sync_failure",
            ));
        }
        unsafe {
            fs_events::FSEventStreamFlushSync(self.stream.stream as fs_events::FSEventStreamRef);
        }
        let sentinel_event = self.wait_for_sentinel(&sentinel_path, EvidenceFlags::DELETE)?;
        let provider_event_id = unsafe {
            fs_events::FSEventStreamGetLatestEventId(
                self.stream.stream as fs_events::ConstFSEventStreamRef,
            )
        }
        .max(sentinel_event.provider_event_id);
        let (response_tx, response_rx) = mpsc::sync_channel(1);
        if self
            .commands
            .send(DurabilityCommand::Fence {
                provider_event_id,
                nonce: nonce.clone(),
                response: response_tx,
            })
            .is_err()
        {
            self.shared
                .revoke("fsevents_durability_worker_disconnected");
            return Err(reconcile_error("fsevents_durability_worker_disconnected"));
        }
        let (public, durable_cut) = match response_rx.recv_timeout(FENCE_TIMEOUT) {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => {
                self.shared.revoke("fsevents_durable_fence_failure");
                return Err(reconcile_error("fsevents_durable_fence_failure"));
            }
            Err(_) => {
                self.shared.revoke("fsevents_durable_fence_timeout");
                return Err(reconcile_error("fsevents_durable_fence_timeout"));
            }
        };
        self.root_identity()?;
        let issued = IssuedFence {
            public: public.clone(),
            expected: expected.clone(),
            root_identity: self.root_identity.clone(),
            owner_token: self.owner_token.clone(),
            owner_fence_nonce: self.owner_fence_nonce.clone(),
            provider_event_id,
            durable_cut,
            kind,
        };
        self.shared.lock().issued_fences.insert(nonce, issued);
        Ok(public)
    }

    fn fail_fence(&self, sentinel_name: &str, reason: &'static str) -> Error {
        let _ = unlinkat(
            self.fence_directory.file(),
            Path::new(sentinel_name),
            AtFlags::empty(),
        );
        let _ = fsync(self.fence_directory.file());
        self.shared.revoke(reason);
        reconcile_error(reason)
    }

    fn wait_for_sentinel(
        &self,
        path: &LedgerPath,
        required: EvidenceFlags,
    ) -> Result<DurableEvent> {
        let deadline = Instant::now() + FENCE_TIMEOUT;
        let mut state = self.shared.lock();
        loop {
            if let Some(reason) = &state.revoked {
                return Err(reconcile_error(reason));
            }
            if let Some(event) = state.events.iter().find(|event| {
                event.event.path == *path && event.event.flags.0 & required.0 == required.0
            }) {
                return Ok(event.clone());
            }
            let now = Instant::now();
            if now >= deadline {
                drop(state);
                self.shared.revoke("fsevents_sentinel_delivery_timeout");
                return Err(reconcile_error("fsevents_sentinel_delivery_timeout"));
            }
            let waited = self
                .shared
                .changed
                .wait_timeout(state, deadline.saturating_duration_since(now))
                .unwrap_or_else(|poison| poison.into_inner());
            state = waited.0;
        }
    }

    fn issued_fence(&self, expected: &ExpectedScope, fence: &ObserverFence) -> Result<IssuedFence> {
        let state = self.shared.lock();
        let Some(issued) = state.issued_fences.get(&fence.nonce) else {
            drop(state);
            self.shared.revoke("fsevents_fence_unknown_or_replayed");
            return Err(reconcile_error("fsevents_fence_unknown_or_replayed"));
        };
        if issued.public != *fence
            || issued.expected != *expected
            || issued.root_identity != self.root_identity
            || issued.owner_token != self.owner_token
            || issued.owner_fence_nonce != self.owner_fence_nonce
            || issued.durable_cut.last_sequence != fence.sequence
            || issued.durable_cut.durable_end_offset != fence.durable_offset
        {
            drop(state);
            self.shared.revoke("fsevents_fence_authentication_mismatch");
            return Err(reconcile_error("fsevents_fence_authentication_mismatch"));
        }
        Ok(issued.clone())
    }

    fn shutdown_inner(&self) -> Result<()> {
        self.shared.shutdown.store(true, Ordering::Release);
        unsafe {
            fs_events::core_foundation::CFRunLoopStop(
                self.stream.run_loop as fs_events::core_foundation::CFRunLoopRef,
            );
        }
        let mut workers = std::mem::take(
            &mut *self
                .workers
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()),
        );
        if !workers.is_empty() {
            workers
                .remove(0)
                .join()
                .map_err(|_| Error::InvalidInput("macOS FSEvents run loop panicked".into()))?;
        }
        let _ = self.commands.send(DurabilityCommand::Shutdown);
        for worker in workers {
            worker.join().map_err(|_| {
                Error::InvalidInput("macOS observer durability worker panicked".into())
            })?;
        }
        self.shared.lock().active = false;
        Ok(())
    }

    #[cfg(debug_assertions)]
    fn inject_flags(&self, flags: u32) -> Result<()> {
        classify_authority_flags(&self.shared, flags)
    }

    #[cfg(debug_assertions)]
    fn stop_durability_worker_for_test(&self) -> Result<()> {
        self.commands
            .send(DurabilityCommand::StopForTest)
            .map_err(|_| reconcile_error("fsevents_durability_worker_disconnected"))
    }

    #[cfg(debug_assertions)]
    fn set_next_fence_nonce_for_test(&self, nonce: Vec<u8>) {
        *self
            .next_test_fence_nonce
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = Some(nonce);
    }

    #[cfg(debug_assertions)]
    fn fail_next_fence_sync_for_test(&self) {
        *self
            .fail_next_fence_sync
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = true;
    }
}

impl QualifiedObserver for MacOsFseventsObserver {
    fn begin_observation(&self, expected: &ExpectedScope) -> Result<ObserverFence> {
        self.flush_fence(expected, IssuedFenceKind::Start)
    }

    fn end_fence(&self, expected: &ExpectedScope, start: &ObserverFence) -> Result<ObserverFence> {
        let issued = self.issued_fence(expected, start)?;
        if !matches!(issued.kind, IssuedFenceKind::Start) {
            self.shared.revoke("fsevents_start_fence_kind_mismatch");
            return Err(reconcile_error("fsevents_start_fence_kind_mismatch"));
        }
        let end = self.flush_fence(
            expected,
            IssuedFenceKind::End {
                start_nonce: start.nonce.clone(),
            },
        )?;
        if end.sequence <= start.sequence || end.durable_offset < start.durable_offset {
            self.shared.revoke("fsevents_non_monotonic_fence");
            return Err(reconcile_error("fsevents_non_monotonic_fence"));
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
            self.shared.revoke("fsevents_root_identity_mismatch");
            return Err(reconcile_error("fsevents_root_identity_mismatch"));
        }
        let issued_start = self.issued_fence(expected, start)?;
        let issued_end = self.issued_fence(expected, end)?;
        if !matches!(issued_start.kind, IssuedFenceKind::Start)
            || !matches!(
                &issued_end.kind,
                IssuedFenceKind::End { start_nonce } if *start_nonce == start.nonce
            )
            || issued_end.provider_event_id < issued_start.provider_event_id
        {
            self.shared.revoke("fsevents_fence_interval_mismatch");
            return Err(reconcile_error("fsevents_fence_interval_mismatch"));
        }
        let events = {
            let state = self.shared.lock();
            state
                .events
                .iter()
                .filter(|item| {
                    !item.internal_fence
                        && item.event.sequence > start.sequence
                        && item.event.sequence <= end.sequence
                        && item.provider_event_id <= issued_end.provider_event_id
                })
                .map(|item| item.event.clone())
                .collect::<Vec<_>>()
        };
        for event in events {
            sink(event)?;
        }
        let qualification = ObserverQualification::native(
            expected,
            root_handle_identity.to_vec(),
            start.clone(),
            end.clone(),
            self.owner_token.clone(),
            self.owner_fence_nonce.clone(),
            issued_end.durable_cut.segment_id.clone(),
            issued_end.durable_cut.durable_end_offset,
            issued_end.durable_cut.durable_end_offset,
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

impl Drop for MacOsFseventsObserver {
    fn drop(&mut self) {
        let _ = self.shutdown_inner();
    }
}

extern "C" fn release_callback_context(info: *const c_void) {
    if !info.is_null() {
        unsafe {
            drop(Box::from_raw(info as *mut CallbackContext));
        }
    }
}

extern "C" fn callback(
    _stream: fs_events::FSEventStreamRef,
    info: *mut c_void,
    count: usize,
    event_paths: *mut c_void,
    event_flags: *const u32,
    event_ids: *const u64,
) {
    if info.is_null() || event_paths.is_null() || event_flags.is_null() || event_ids.is_null() {
        return;
    }
    let context = unsafe { &*(info as *const CallbackContext) };
    let paths = unsafe { std::slice::from_raw_parts(event_paths as *const *const i8, count) };
    let flags = unsafe { std::slice::from_raw_parts(event_flags, count) };
    let ids = unsafe { std::slice::from_raw_parts(event_ids, count) };
    for index in 0..count {
        if classify_authority_flags(&context.shared, flags[index]).is_err() {
            return;
        }
        if flags[index] & fs_events::kFSEventStreamEventFlagHistoryDone != 0 {
            continue;
        }
        let Some(path_ptr) = paths.get(index).copied().filter(|path| !path.is_null()) else {
            context.shared.revoke("fsevents_path_decode_ambiguity");
            return;
        };
        let Ok(path_text) = unsafe { CStr::from_ptr(path_ptr) }.to_str() else {
            context.shared.revoke("fsevents_path_decode_ambiguity");
            return;
        };
        let Ok(path) = normalize_callback_path(&context.root_path, Path::new(path_text)) else {
            context.shared.revoke("fsevents_path_escaped_or_ambiguous");
            return;
        };
        if observer_internal_path(&path)
            && !observer_fence_path(&path)
            && !policy_dependency_event(&context.policy_dependencies, &path)
        {
            continue;
        }
        let evidence = evidence_flags(flags[index]);
        if evidence.0 == 0 {
            continue;
        }
        match context.records.try_send(DurabilityCommand::Record {
            path,
            flags: evidence,
            provider_event_id: ids[index],
        }) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                context
                    .shared
                    .revoke("fsevents_bounded_callback_queue_overflow");
                return;
            }
            Err(TrySendError::Disconnected(_)) => {
                context
                    .shared
                    .revoke("fsevents_durability_worker_disconnected");
                return;
            }
        }
    }
}

fn classify_authority_flags(shared: &Shared, flags: u32) -> Result<()> {
    if flags & GAP_FLAGS != 0 {
        let reason = if flags & fs_events::kFSEventStreamEventFlagMustScanSubDirs != 0 {
            "fsevents_must_scan_subdirs"
        } else if flags & fs_events::kFSEventStreamEventFlagUserDropped != 0 {
            "fsevents_user_dropped"
        } else if flags & fs_events::kFSEventStreamEventFlagKernelDropped != 0 {
            "fsevents_kernel_dropped"
        } else if flags & fs_events::kFSEventStreamEventFlagEventIdsWrapped != 0 {
            "fsevents_event_ids_wrapped"
        } else if flags & fs_events::kFSEventStreamEventFlagRootChanged != 0 {
            "fsevents_root_changed"
        } else {
            "fsevents_unmount"
        };
        shared.revoke(reason);
        return Err(reconcile_error(reason));
    }
    if flags & fs_events::kFSEventStreamEventFlagHistoryDone != 0 {
        let mut state = shared.lock();
        if !state.history_required || state.history_done {
            drop(state);
            shared.revoke("fsevents_inconsistent_history_done");
            return Err(reconcile_error("fsevents_inconsistent_history_done"));
        }
        state.history_done = true;
        shared.changed.notify_all();
    }
    Ok(())
}

fn normalize_callback_path(root: &Path, event_path: &Path) -> Result<LedgerPath> {
    if !event_path.is_absolute() {
        return Err(Error::InvalidInput("FSEvents path was not absolute".into()));
    }
    let relative = event_path
        .strip_prefix(root)
        .map_err(|_| Error::InvalidInput("FSEvents path escaped pinned root".into()))?;
    if relative.as_os_str().is_empty()
        || relative
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(Error::InvalidInput(
            "FSEvents path was not a normalized descendant".into(),
        ));
    }
    let text = relative
        .to_str()
        .ok_or_else(|| Error::InvalidInput("FSEvents path was not UTF-8".into()))?;
    LedgerPath::parse(text)
}

fn evidence_flags(flags: u32) -> EvidenceFlags {
    let mut evidence = EvidenceFlags::default();
    if flags & fs_events::kFSEventStreamEventFlagItemCreated != 0 {
        evidence |= EvidenceFlags::CREATE;
    }
    if flags & fs_events::kFSEventStreamEventFlagItemRemoved != 0 {
        evidence |= EvidenceFlags::DELETE;
    }
    if flags & fs_events::kFSEventStreamEventFlagItemModified != 0 {
        evidence |= EvidenceFlags::CONTENT;
    }
    if flags
        & (fs_events::kFSEventStreamEventFlagItemInodeMetaMod
            | fs_events::kFSEventStreamEventFlagItemFinderInfoMod
            | fs_events::kFSEventStreamEventFlagItemChangeOwner
            | fs_events::kFSEventStreamEventFlagItemXattrMod)
        != 0
    {
        evidence |= EvidenceFlags::MODE;
    }
    if flags & fs_events::kFSEventStreamEventFlagItemRenamed != 0 {
        // FSEvents does not provide a rename cookie. Marking each delivered
        // endpoint both ways is conservative and retains every endpoint.
        evidence |= EvidenceFlags::RENAME_FROM | EvidenceFlags::RENAME_TO;
    }
    if flags & fs_events::kFSEventStreamEventFlagItemIsDir != 0
        && flags
            & (fs_events::kFSEventStreamEventFlagItemCreated
                | fs_events::kFSEventStreamEventFlagItemRenamed)
            != 0
    {
        evidence |= EvidenceFlags::PROVIDER_COMPLETE_PREFIX;
    }
    evidence
}

fn observer_internal_path(path: &LedgerPath) -> bool {
    let path = path.as_str();
    path == ".trail" || path.starts_with(".trail/") || path == ".git" || path.starts_with(".git/")
}

fn observer_fence_path(path: &LedgerPath) -> bool {
    path.as_str().starts_with(".trail/observer-fences/")
}

fn normalize_policy_dependencies(
    root: &Path,
    requested_root: &Path,
    dependencies: &[PathBuf],
) -> Result<Vec<LedgerPath>> {
    dependencies
        .iter()
        .map(|dependency| {
            let relative = dependency
                .strip_prefix(root)
                .or_else(|_| dependency.strip_prefix(requested_root))
                .map_err(|_| {
                    reconcile_error("fsevents_external_policy_dependency_is_unobservable")
                })?;
            let text = relative.to_str().ok_or_else(|| {
                reconcile_error("fsevents_policy_dependency_path_decode_ambiguity")
            })?;
            LedgerPath::parse(text)
        })
        .collect()
}

fn policy_dependency_event(dependencies: &[LedgerPath], path: &LedgerPath) -> bool {
    dependencies.iter().any(|dependency| {
        dependency == path
            || dependency
                .as_str()
                .strip_prefix(path.as_str())
                .is_some_and(|suffix| suffix.starts_with('/'))
    })
}

fn run_stream(
    root_path: PathBuf,
    since_when: u64,
    callback_context: Box<CallbackContext>,
    ready: SyncSender<Result<StreamHandle>>,
    shared: Arc<Shared>,
) {
    let Some(root_text) = root_path.to_str() else {
        let _ = ready.send(Err(reconcile_error("fsevents_non_utf8_root")));
        return;
    };
    let mut cf_error = ptr::null_mut();
    let cf_path =
        unsafe { fs_events::core_foundation::str_path_to_cfstring_ref(root_text, &mut cf_error) };
    if cf_path.is_null() {
        if !cf_error.is_null() {
            unsafe { fs_events::core_foundation::CFRelease(cf_error.cast()) };
        }
        let _ = ready.send(Err(reconcile_error("fsevents_root_cfstring_failure")));
        return;
    }
    let paths = unsafe {
        fs_events::core_foundation::CFArrayCreateMutable(
            fs_events::core_foundation::kCFAllocatorDefault,
            1,
            &fs_events::core_foundation::kCFTypeArrayCallBacks,
        )
    };
    if paths.is_null() {
        unsafe { fs_events::core_foundation::CFRelease(cf_path) };
        let _ = ready.send(Err(reconcile_error("fsevents_paths_array_failure")));
        return;
    }
    unsafe {
        fs_events::core_foundation::CFArrayAppendValue(paths, cf_path);
        fs_events::core_foundation::CFRelease(cf_path);
    }
    let raw_context = Box::into_raw(callback_context);
    let context = fs_events::FSEventStreamContext {
        version: 0,
        info: raw_context.cast(),
        retain: None,
        release: Some(release_callback_context),
        copy_description: None,
    };
    let stream = unsafe {
        fs_events::FSEventStreamCreate(
            fs_events::core_foundation::kCFAllocatorDefault,
            callback,
            &context,
            paths,
            since_when,
            0.01,
            STREAM_FLAGS,
        )
    };
    unsafe { fs_events::core_foundation::CFRelease(paths) };
    if stream.is_null() {
        unsafe { drop(Box::from_raw(raw_context)) };
        let _ = ready.send(Err(reconcile_error("fsevents_stream_create_failure")));
        return;
    }
    unsafe {
        let run_loop = fs_events::core_foundation::CFRunLoopGetCurrent();
        fs_events::FSEventStreamScheduleWithRunLoop(
            stream,
            run_loop,
            fs_events::core_foundation::kCFRunLoopDefaultMode,
        );
        if fs_events::FSEventStreamStart(stream) == 0 {
            fs_events::FSEventStreamInvalidate(stream);
            fs_events::FSEventStreamRelease(stream);
            let _ = ready.send(Err(reconcile_error("fsevents_stream_start_failure")));
            return;
        }
        if ready
            .send(Ok(StreamHandle {
                stream: stream as usize,
                run_loop: run_loop as usize,
            }))
            .is_err()
        {
            fs_events::FSEventStreamStop(stream);
            fs_events::FSEventStreamInvalidate(stream);
            fs_events::FSEventStreamRelease(stream);
            return;
        }
        fs_events::core_foundation::CFRunLoopRun();
        fs_events::FSEventStreamStop(stream);
        fs_events::FSEventStreamInvalidate(stream);
        fs_events::FSEventStreamRelease(stream);
    }
    if !shared.shutdown.load(Ordering::Acquire) {
        shared.revoke("fsevents_run_loop_stopped");
    }
}

fn run_durability_worker(
    receiver: Receiver<DurabilityCommand>,
    mut durability: Box<dyn MacObserverDurability>,
    shared: Arc<Shared>,
    cursor_template: MacOsProviderCursor,
) {
    while let Ok(command) = receiver.recv() {
        if matches!(command, DurabilityCommand::Shutdown) {
            return;
        }
        #[cfg(debug_assertions)]
        if matches!(command, DurabilityCommand::StopForTest) {
            shared.revoke("fsevents_durability_worker_stopped");
            return;
        }
        let (path, flags, provider_event_id, internal, fence_nonce, response) = match command {
            DurabilityCommand::Record {
                path,
                flags,
                provider_event_id,
            } => {
                let internal = path.as_str().starts_with(".trail/observer-fences/");
                (path, flags, provider_event_id, internal, Vec::new(), None)
            }
            DurabilityCommand::Fence {
                provider_event_id,
                nonce,
                response,
            } => {
                let name = format!(".trail-fsevents-fence-{}", hex::encode(&nonce));
                let Ok(path) = LedgerPath::parse(&name) else {
                    let _ = response.send(Err(reconcile_error("fsevents_fence_path_failure")));
                    shared.revoke("fsevents_fence_path_failure");
                    return;
                };
                (
                    path,
                    EvidenceFlags::default(),
                    provider_event_id,
                    true,
                    nonce,
                    Some(response),
                )
            }
            #[cfg(debug_assertions)]
            DurabilityCommand::StopForTest => unreachable!(),
            DurabilityCommand::Shutdown => unreachable!(),
        };
        let sequence = shared.lock().next_sequence;
        let mut cursor = cursor_template.clone();
        cursor.event_id = provider_event_id;
        let provider_cursor = match cursor.encode() {
            Ok(cursor) => cursor,
            Err(error) => {
                shared.revoke(format!("fsevents_cursor_encode_failure:{error}"));
                return;
            }
        };
        let record = ObserverRecord {
            sequence,
            source: EvidenceSource::Observer,
            path: path.clone(),
            flags,
            provider_cursor,
        };
        let cut = match durability.append_and_flush(record) {
            Ok(cut) => cut,
            Err(error) => {
                if let Some(response) = response {
                    let _ = response.send(Err(reconcile_error("fsevents_durability_failure")));
                }
                shared.revoke(format!("fsevents_durability_failure:{error}"));
                return;
            }
        };
        let public = ObserverFence {
            sequence,
            durable_offset: cut.durable_end_offset,
            nonce: fence_nonce,
        };
        {
            let mut state = shared.lock();
            if provider_event_id < state.last_provider_event_id {
                drop(state);
                shared.revoke("fsevents_non_monotonic_provider_cursor");
                return;
            }
            state.next_sequence = sequence.saturating_add(1);
            state.last_provider_event_id = provider_event_id;
            state.last_cursor = Some(cursor);
            state.events.push(DurableEvent {
                event: ObserverEvent {
                    path,
                    flags,
                    sequence,
                },
                provider_event_id,
                cut: cut.clone(),
                internal_fence: internal,
            });
            if state.events.len() > MAX_RETAINED_EVENTS {
                drop(state);
                shared.revoke("fsevents_retained_event_overflow");
                return;
            }
            shared.changed.notify_all();
        }
        if let Some(response) = response {
            let _ = response.send(Ok((public, cut)));
        }
    }
    if !shared.shutdown.load(Ordering::Acquire) {
        shared.revoke("fsevents_durability_worker_disconnected");
    }
}

fn native_capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        durable_cursor: true,
        linearizable_fence: true,
        rename_pairing: false,
        overflow_scope: true,
        filesystem_supported: true,
        clean_proof_allowed: true,
        power_loss_durability: true,
    }
}

fn open_root_no_follow(path: &Path) -> Result<File> {
    Ok(OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?)
}

fn root_identity(file: &File) -> Result<Vec<u8>> {
    let metadata = file.metadata()?;
    Ok(format!(
        "mac-root-v1:dev={};ino={};mode={};uid={};gid={}",
        metadata.dev(),
        metadata.ino(),
        metadata.mode(),
        metadata.uid(),
        metadata.gid()
    )
    .into_bytes())
}

#[cfg(debug_assertions)]
fn ensure_apfs(path: &Path) -> Result<()> {
    let path = std::ffi::CString::new(path.as_os_str().as_bytes())
        .map_err(|_| Error::InvalidInput("APFS fixture path contained NUL".into()))?;
    let mut stat: libc::statfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statfs(path.as_ptr(), &mut stat) } != 0 {
        return Err(Error::Io(std::io::Error::last_os_error()));
    }
    let filesystem = unsafe { CStr::from_ptr(stat.f_fstypename.as_ptr()) }
        .to_str()
        .map_err(|_| Error::InvalidInput("filesystem type was not UTF-8".into()))?;
    if filesystem != "apfs" {
        return Err(Error::InvalidInput(format!(
            "real macOS observer qualification requires APFS, found {filesystem}"
        )));
    }
    Ok(())
}

fn reconcile_error(reason: &str) -> Error {
    Error::ChangeLedgerReconcileRequired {
        scope: "native-macos-observer".into(),
        state: "untrusted_gap".into(),
        reason: reason.into(),
        command: "trail status".into(),
    }
}

#[cfg(debug_assertions)]
struct MemoryDurability {
    binding: ObserverWriterBinding,
    offset: u64,
    delay: Duration,
    records: Arc<Mutex<Vec<ObserverRecord>>>,
}

#[cfg(debug_assertions)]
impl MacObserverDurability for MemoryDurability {
    fn binding(&self) -> ObserverWriterBinding {
        self.binding.clone()
    }

    fn append_and_flush(&mut self, record: ObserverRecord) -> Result<DurableCut> {
        if record.sequence != self.offset.saturating_add(1) {
            return Err(Error::InvalidInput(
                "test durability received non-contiguous sequence".into(),
            ));
        }
        if !self.delay.is_zero() {
            thread::sleep(self.delay);
        }
        self.offset = self.offset.saturating_add(1);
        let provider_cursor = record.provider_cursor.clone();
        self.records
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(record.clone());
        Ok(DurableCut {
            segment_id: "macos-native-test".into(),
            durable_end_offset: self.offset,
            last_sequence: record.sequence,
            last_hash: [0; 32],
            provider_cursor,
        })
    }
}

#[cfg(debug_assertions)]
fn memory_durability(
    provider_identity: Vec<u8>,
    delay: Duration,
) -> (MemoryDurability, Arc<Mutex<Vec<ObserverRecord>>>) {
    let records = Arc::new(Mutex::new(Vec::new()));
    (
        MemoryDurability {
            binding: ObserverWriterBinding {
                owner_token: hex::encode([0x71; 32]),
                provider_id: hex::encode(&provider_identity),
                provider_identity,
                fence_nonce: vec![0x72; 24],
            },
            offset: 0,
            delay,
            records: Arc::clone(&records),
        },
        records,
    )
}

#[cfg(debug_assertions)]
struct TestFixture {
    temp: tempfile::TempDir,
    observer: MacOsFseventsObserver,
    expected: ExpectedScope,
    records: Arc<Mutex<Vec<ObserverRecord>>>,
}

#[cfg(debug_assertions)]
struct NativeSegmentFixture {
    temp: tempfile::TempDir,
    db: Trail,
    expected: ExpectedScope,
    segment_directory: PathBuf,
}

#[cfg(debug_assertions)]
impl NativeSegmentFixture {
    fn new() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false)?;
        let db = Trail::open(temp.path())?;
        let branch = db.current_branch()?;
        let head = db.resolve_branch_ref(&branch)?;
        let scope = ScopeIdentity {
            scope_id: ScopeId([0xb8; 32]),
            kind: ScopeKind::Workspace,
            owner_id: "macos-native-segment".into(),
        };
        let provider_identity = b"macos-fsevents-segment-writer-v1".to_vec();
        let filesystem_identity = root_identity(&open_root_no_follow(temp.path())?)?;
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
                fingerprint: [0xb9; 32],
                generation: 1,
            },
            &FilesystemIdentity(filesystem_identity.clone()),
            &ProviderIdentity {
                identity: provider_identity.clone(),
                capabilities: native_capabilities(),
            },
        )?;
        let expected = ExpectedScope {
            scope_id: scope.scope_id,
            epoch: 1,
            ref_name: baseline.ref_name,
            ref_generation: baseline.ref_generation,
            baseline_root: baseline.root_id,
            policy_fingerprint: [0xb9; 32],
            policy_generation: 1,
            filesystem_identity,
            provider_identity,
        };
        let segment_directory = db.db_dir.join("change-observer-segments");
        Ok(Self {
            temp,
            db,
            expected,
            segment_directory,
        })
    }

    fn observer(&self) -> Result<MacOsFseventsObserver> {
        let writer = SegmentWriter::acquire(
            &self.db.sqlite_path,
            &self.segment_directory,
            self.expected.scope_id,
            self.expected.epoch,
            [0xba; 32],
            &hex::encode(&self.expected.provider_identity),
            Vec::new(),
            Duration::from_secs(3_600),
        )?;
        let durability = MacSegmentWriterDurability::new(
            writer,
            self.expected.provider_identity.clone(),
            vec![0xbb; 24],
        )?;
        MacOsFseventsObserver::start(
            self.temp.path(),
            Box::new(durability),
            None,
            &[self.temp.path().join(".trail/config.toml")],
        )
    }
}

#[cfg(debug_assertions)]
impl TestFixture {
    fn new() -> Result<Self> {
        let temp = tempfile::tempdir()?;
        std::fs::create_dir(temp.path().join(".trail"))?;
        Self::at(temp, None, Duration::ZERO)
    }

    fn at(
        temp: tempfile::TempDir,
        resume: Option<MacOsProviderCursor>,
        delay: Duration,
    ) -> Result<Self> {
        let provider_identity = b"macos-fsevents-file-events-v1".to_vec();
        let (durability, records) = memory_durability(provider_identity.clone(), delay);
        let observer = MacOsFseventsObserver::start(
            temp.path(),
            Box::new(durability),
            resume,
            &[temp.path().join(".trail/config.toml")],
        )?;
        let expected = ExpectedScope {
            scope_id: crate::db::change_ledger::ScopeId([0xa8; 32]),
            epoch: 1,
            ref_name: "refs/branches/main".into(),
            ref_generation: 1,
            baseline_root: crate::ObjectId("object_macos_observer".into()),
            policy_fingerprint: [0xa9; 32],
            policy_generation: 1,
            filesystem_identity: observer.root_identity.clone(),
            provider_identity,
        };
        Ok(Self {
            temp,
            observer,
            expected,
            records,
        })
    }

    fn interval(&self, action: impl FnOnce(&Path) -> Result<()>) -> Result<Vec<ObserverEvent>> {
        let start = self.observer.begin_observation(&self.expected)?;
        action(self.temp.path())?;
        let end = self.observer.end_fence(&self.expected, &start)?;
        let mut events = Vec::new();
        self.observer.drain_through(
            &self.expected,
            &self.observer.root_identity,
            &start,
            &end,
            &mut |event| {
                events.push(event);
                Ok(())
            },
        )?;
        Ok(events)
    }
}

#[cfg(debug_assertions)]
fn has_event(events: &[ObserverEvent], path: &str, flag: EvidenceFlags) -> bool {
    events
        .iter()
        .any(|event| event.path.as_str() == path && event.flags.0 & flag.0 != 0)
}

#[cfg(debug_assertions)]
pub(crate) fn run_real_apfs_file_events() -> std::result::Result<(), String> {
    fn run() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        let fixture = TestFixture::new()?;
        ensure_apfs(fixture.temp.path())?;
        let events = fixture.interval(|root| {
            std::fs::write(root.join("tracked.txt"), b"one")?;
            Ok(())
        })?;
        if !has_event(&events, "tracked.txt", EvidenceFlags::CREATE) {
            return Err(Error::Corrupt("FSEvents omitted file create".into()));
        }
        let events = fixture.interval(|root| {
            std::fs::write(root.join("tracked.txt"), b"two")?;
            std::fs::set_permissions(
                root.join("tracked.txt"),
                std::fs::Permissions::from_mode(0o700),
            )?;
            Ok(())
        })?;
        if !has_event(&events, "tracked.txt", EvidenceFlags::CONTENT)
            || !has_event(&events, "tracked.txt", EvidenceFlags::MODE)
        {
            return Err(Error::Corrupt(
                "FSEvents omitted content or mode evidence".into(),
            ));
        }
        let events = fixture.interval(|root| {
            std::fs::rename(root.join("tracked.txt"), root.join("renamed.txt"))?;
            Ok(())
        })?;
        for path in ["tracked.txt", "renamed.txt"] {
            if !has_event(&events, path, EvidenceFlags::RENAME_FROM)
                || !has_event(&events, path, EvidenceFlags::RENAME_TO)
            {
                return Err(Error::Corrupt(format!(
                    "FSEvents omitted conservative rename endpoint {path}"
                )));
            }
        }
        let events = fixture.interval(|root| {
            std::fs::create_dir(root.join("dir-a"))?;
            std::fs::write(root.join("dir-a/child"), b"child")?;
            std::fs::rename(root.join("dir-a"), root.join("dir-b"))?;
            std::fs::write(root.join("case"), b"case")?;
            std::fs::rename(root.join("case"), root.join("CASE"))?;
            Ok(())
        })?;
        for path in ["dir-a", "dir-b", "case", "CASE"] {
            if !has_event(&events, path, EvidenceFlags::RENAME_FROM) {
                return Err(Error::Corrupt(format!(
                    "FSEvents omitted directory/case rename endpoint {path}"
                )));
            }
        }
        let events = fixture.interval(|root| {
            std::fs::remove_file(root.join("renamed.txt"))?;
            Ok(())
        })?;
        if !has_event(&events, "renamed.txt", EvidenceFlags::DELETE) {
            return Err(Error::Corrupt("FSEvents omitted file delete".into()));
        }
        let events = fixture.interval(|root| {
            for index in 0..256 {
                std::fs::write(root.join(format!("batch-{index}")), b"batch")?;
            }
            Ok(())
        })?;
        if !has_event(&events, "batch-255", EvidenceFlags::CREATE) {
            return Err(Error::Corrupt(
                "synchronous flush omitted delayed batch tail".into(),
            ));
        }
        let events = fixture.interval(|root| {
            std::fs::write(
                root.join(".trail/config.toml"),
                b"ignore_gitignored = true\n",
            )?;
            std::fs::write(root.join(".trail/internal-noise"), b"noise")?;
            Ok(())
        })?;
        if !has_event(&events, ".trail/config.toml", EvidenceFlags::CREATE)
            || events
                .iter()
                .any(|event| event.path.as_str() == ".trail/internal-noise")
        {
            return Err(Error::Corrupt(
                "internal policy dependency was lost or storage noise leaked".into(),
            ));
        }
        if fixture.records.lock().unwrap().is_empty() {
            return Err(Error::Corrupt(
                "durability worker appended no records".into(),
            ));
        }
        Ok(())
    }
    run().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_gap_flag_matrix() -> std::result::Result<(), String> {
    fn run() -> Result<()> {
        for (flag, reason) in [
            (
                fs_events::kFSEventStreamEventFlagMustScanSubDirs,
                "fsevents_must_scan_subdirs",
            ),
            (
                fs_events::kFSEventStreamEventFlagUserDropped,
                "fsevents_user_dropped",
            ),
            (
                fs_events::kFSEventStreamEventFlagKernelDropped,
                "fsevents_kernel_dropped",
            ),
            (
                fs_events::kFSEventStreamEventFlagEventIdsWrapped,
                "fsevents_event_ids_wrapped",
            ),
            (
                fs_events::kFSEventStreamEventFlagRootChanged,
                "fsevents_root_changed",
            ),
            (
                fs_events::kFSEventStreamEventFlagUnmount,
                "fsevents_unmount",
            ),
        ] {
            let fixture = TestFixture::new()?;
            let error = fixture.observer.inject_flags(flag).unwrap_err().to_string();
            if !error.contains(reason)
                || fixture.observer.ensure_available().unwrap_err().code()
                    != "CHANGE_LEDGER_RECONCILE_REQUIRED"
            {
                return Err(Error::Corrupt(format!(
                    "gap flag did not globally revoke qualification: {reason}"
                )));
            }
        }
        let fixture = TestFixture::new()?;
        let error = fixture
            .observer
            .inject_flags(fs_events::kFSEventStreamEventFlagHistoryDone)
            .unwrap_err();
        if !error
            .to_string()
            .contains("fsevents_inconsistent_history_done")
        {
            return Err(Error::Corrupt(
                "unexpected HistoryDone did not fail closed".into(),
            ));
        }
        Ok(())
    }
    run().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
fn callback_overflow_or_disconnect(disconnect: bool) -> Result<()> {
    let temp = tempfile::tempdir()?;
    let shared = Arc::new(Shared {
        state: Mutex::new(State {
            active: true,
            revoked: None,
            history_required: false,
            history_done: true,
            events: Vec::new(),
            next_sequence: 1,
            last_provider_event_id: 0,
            last_cursor: None,
            issued_fences: HashMap::new(),
        }),
        changed: Condvar::new(),
        shutdown: AtomicBool::new(false),
    });
    let (tx, rx) = mpsc::sync_channel(1);
    if disconnect {
        drop(rx);
    } else {
        tx.send(DurabilityCommand::Shutdown)
            .map_err(|_| Error::InvalidInput("could not prime bounded queue".into()))?;
    }
    let context = CallbackContext {
        root_path: temp.path().to_path_buf(),
        policy_dependencies: Vec::new(),
        records: tx,
        shared: Arc::clone(&shared),
    };
    let path = std::ffi::CString::new(temp.path().join("overflow").to_string_lossy().as_bytes())
        .map_err(|_| Error::InvalidInput("test callback path contained NUL".into()))?;
    let paths = [path.as_ptr()];
    let flags = [fs_events::kFSEventStreamEventFlagItemCreated
        | fs_events::kFSEventStreamEventFlagItemIsFile];
    let ids = [1_u64];
    callback(
        ptr::null_mut(),
        (&context as *const CallbackContext).cast_mut().cast(),
        1,
        paths.as_ptr().cast_mut().cast(),
        flags.as_ptr(),
        ids.as_ptr(),
    );
    let reason = shared
        .lock()
        .revoked
        .clone()
        .ok_or_else(|| Error::Corrupt("bounded callback failure did not revoke".into()))?;
    let expected = if disconnect {
        "fsevents_durability_worker_disconnected"
    } else {
        "fsevents_bounded_callback_queue_overflow"
    };
    if reason != expected {
        return Err(Error::Corrupt(format!(
            "bounded callback revoked for {reason}, expected {expected}"
        )));
    }
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn run_continuity_fault_matrix() -> std::result::Result<(), String> {
    if let Ok(root) = std::env::var("TRAIL_MACOS_OBSERVER_OWNER_CHILD_ROOT") {
        return run_owner_process_child(Path::new(&root)).map_err(|error| error.to_string());
    }
    fn run() -> Result<()> {
        use std::os::unix::fs::PermissionsExt;

        callback_overflow_or_disconnect(false)?;
        callback_overflow_or_disconnect(true)?;

        let external_root = tempfile::tempdir()?;
        std::fs::create_dir(external_root.path().join(".trail"))?;
        let external_dependency = tempfile::NamedTempFile::new()?;
        let provider = b"macos-fsevents-file-events-v1".to_vec();
        let (durability, _) = memory_durability(provider, Duration::ZERO);
        let error = match MacOsFseventsObserver::start(
            external_root.path(),
            Box::new(durability),
            None,
            &[external_dependency.path().to_path_buf()],
        ) {
            Ok(_) => {
                return Err(Error::Corrupt(
                    "unobservable external policy dependency was accepted".into(),
                ));
            }
            Err(error) => error,
        };
        if !error
            .to_string()
            .contains("fsevents_external_policy_dependency_is_unobservable")
        {
            return Err(Error::Corrupt(
                "external policy dependency did not fail closed".into(),
            ));
        }

        let leftover_temp = tempfile::tempdir()?;
        std::fs::create_dir(leftover_temp.path().join(".trail"))?;
        let leftover_dir = leftover_temp.path().join(".trail/observer-fences");
        std::fs::create_dir(&leftover_dir)?;
        std::fs::set_permissions(&leftover_dir, std::fs::Permissions::from_mode(0o700))?;
        std::fs::write(leftover_dir.join("crash-leftover"), b"old")?;
        let leftover = TestFixture::at(leftover_temp, None, Duration::ZERO)?;
        let events = leftover.interval(|root| {
            std::fs::write(root.join("visible"), b"visible")?;
            Ok(())
        })?;
        if events
            .iter()
            .any(|event| event.path.as_str().starts_with(".trail/observer-fences/"))
            || !has_event(&events, "visible", EvidenceFlags::CREATE)
        {
            return Err(Error::Corrupt(
                "crash-leftover fence leaked into user-visible evidence".into(),
            ));
        }

        let collision = TestFixture::new()?;
        let collision_nonce = vec![0xc1; 24];
        std::fs::write(
            collision.temp.path().join(format!(
                ".trail/observer-fences/{}",
                hex::encode(&collision_nonce)
            )),
            b"hostile",
        )?;
        collision
            .observer
            .set_next_fence_nonce_for_test(collision_nonce);
        let error = collision
            .observer
            .begin_observation(&collision.expected)
            .unwrap_err()
            .to_string();
        if !error.contains("fsevents_fence_collision_or_create_failure")
            || collision.observer.ensure_available().is_ok()
        {
            return Err(Error::Corrupt(
                "fence collision did not globally revoke qualification".into(),
            ));
        }

        let sync_failure = TestFixture::new()?;
        sync_failure.observer.fail_next_fence_sync_for_test();
        let error = sync_failure
            .observer
            .begin_observation(&sync_failure.expected)
            .unwrap_err()
            .to_string();
        if !error.contains("fsevents_fence_file_sync_failure")
            || sync_failure.observer.ensure_available().is_ok()
            || std::fs::read_dir(sync_failure.temp.path().join(".trail/observer-fences"))?
                .next()
                .is_some()
        {
            return Err(Error::Corrupt(
                "fence sync failure did not revoke and clean up".into(),
            ));
        }

        let fixture = TestFixture::new()?;
        fixture.interval(|root| {
            std::fs::write(root.join("restart"), b"before")?;
            Ok(())
        })?;
        let cursor = fixture
            .observer
            .resume_cursor()?
            .ok_or_else(|| Error::Corrupt("durable resume cursor was absent".into()))?;
        let persisted_cursor = {
            let records = fixture
                .records
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            MacOsProviderCursor::decode(
                &records
                    .last()
                    .ok_or_else(|| Error::Corrupt("persisted cursor record was absent".into()))?
                    .provider_cursor,
            )?
        };
        if persisted_cursor != cursor {
            return Err(Error::Corrupt(
                "persisted provider cursor did not round-trip exactly".into(),
            ));
        }
        let root = fixture.temp.path().to_path_buf();
        let temp = fixture.temp;
        drop(fixture.observer);
        let resumed = TestFixture::at(temp, Some(cursor.clone()), Duration::ZERO)?;
        let events = resumed.interval(|root| {
            std::fs::write(root.join("restart"), b"after")?;
            Ok(())
        })?;
        if !has_event(&events, "restart", EvidenceFlags::CONTENT) {
            return Err(Error::Corrupt(
                "resumed stream omitted post-restart content".into(),
            ));
        }
        let mut forged = cursor;
        forged.device = forged.device.saturating_add(1);
        let bogus_temp = tempfile::tempdir()?;
        std::fs::create_dir(bogus_temp.path().join(".trail"))?;
        let provider = b"macos-fsevents-file-events-v1".to_vec();
        let (durability, _) = memory_durability(provider, Duration::ZERO);
        if MacOsFseventsObserver::start(bogus_temp.path(), Box::new(durability), Some(forged), &[])
            .is_ok()
        {
            return Err(Error::Corrupt("forged resume identity was accepted".into()));
        }

        let displaced = root.with_extension("displaced-macos-observer");
        std::fs::rename(&root, &displaced)?;
        std::fs::create_dir(&root)?;
        let error = resumed.observer.root_identity().unwrap_err();
        if error.code() != "CHANGE_LEDGER_RECONCILE_REQUIRED"
            || (!error.to_string().contains("fsevents_root_replaced")
                && !error.to_string().contains("fsevents_root_changed"))
        {
            return Err(Error::Corrupt("root replacement did not revoke".into()));
        }
        std::fs::remove_dir(&root)?;
        std::fs::rename(&displaced, &root)?;

        let fixture = TestFixture::new()?;
        fixture.observer.stop_durability_worker_for_test()?;
        let deadline = Instant::now() + FENCE_TIMEOUT;
        loop {
            if fixture.observer.ensure_available().is_err() {
                break;
            }
            if Instant::now() >= deadline {
                return Err(Error::Corrupt(
                    "durability worker death did not revoke observer".into(),
                ));
            }
            thread::sleep(Duration::from_millis(1));
        }

        run_owner_process_death()?;

        Ok(())
    }
    run().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
fn run_owner_process_child(root: &Path) -> Result<()> {
    use std::io::Write as _;

    let db = Trail::open(root)?;
    let branch = db.current_branch()?;
    let head = db.resolve_branch_ref(&branch)?;
    let expected = ExpectedScope {
        scope_id: ScopeId([0xb8; 32]),
        epoch: 1,
        ref_name: head.name,
        ref_generation: u64::try_from(head.generation)
            .map_err(|_| Error::Corrupt("negative child ref generation".into()))?,
        baseline_root: head.root_id,
        policy_fingerprint: [0xb9; 32],
        policy_generation: 1,
        filesystem_identity: root_identity(&open_root_no_follow(root)?)?,
        provider_identity: b"macos-fsevents-segment-writer-v1".to_vec(),
    };
    let segment_directory = db.db_dir.join("change-observer-segments");
    let writer = SegmentWriter::acquire(
        &db.sqlite_path,
        &segment_directory,
        expected.scope_id,
        expected.epoch,
        [0xbc; 32],
        &hex::encode(&expected.provider_identity),
        Vec::new(),
        Duration::from_secs(3_600),
    )?;
    let durability = MacSegmentWriterDurability::new(
        writer,
        expected.provider_identity.clone(),
        vec![0xbd; 24],
    )?;
    let observer = MacOsFseventsObserver::start(
        root,
        Box::new(durability),
        None,
        &[root.join(".trail/config.toml")],
    )?;
    observer.begin_observation(&expected)?;
    println!("TRAIL_MACOS_OBSERVER_OWNER_READY");
    std::io::stdout().flush()?;
    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

#[cfg(debug_assertions)]
fn run_owner_process_death() -> Result<()> {
    use std::io::{BufRead, BufReader};
    use std::process::{Command, Stdio};

    let fixture = NativeSegmentFixture::new()?;
    let executable = std::env::current_exe()?;
    let mut child = Command::new(executable)
        .arg("fsevents_restart_root_cursor_overflow_and_worker_death_fail_closed")
        .arg("--nocapture")
        .arg("--test-threads=1")
        .env("TRAIL_MACOS_OBSERVER_OWNER_CHILD_ROOT", fixture.temp.path())
        .stdout(Stdio::piped())
        .spawn()?;
    let stdout = child.stdout.take().ok_or_else(|| {
        Error::InvalidInput("macOS observer owner child stdout was unavailable".into())
    })?;
    let mut ready = false;
    for line in BufReader::new(stdout).lines() {
        let line = line?;
        if line.contains("TRAIL_MACOS_OBSERVER_OWNER_READY") {
            ready = true;
            break;
        }
    }
    if !ready {
        let _ = child.kill();
        let _ = child.wait();
        return Err(Error::Corrupt(
            "macOS observer owner child exited before readiness".into(),
        ));
    }
    child.kill()?;
    let status = child.wait()?;
    if status.success() {
        return Err(Error::Corrupt(
            "macOS observer owner child was not killed".into(),
        ));
    }
    let replacement = SegmentWriter::acquire(
        &fixture.db.sqlite_path,
        &fixture.segment_directory,
        fixture.expected.scope_id,
        fixture.expected.epoch,
        [0xbe; 32],
        &hex::encode(&fixture.expected.provider_identity),
        Vec::new(),
        Duration::from_secs(3_600),
    );
    if replacement.is_ok() {
        return Err(Error::Corrupt(
            "same-epoch macOS observer owner replacement succeeded after SIGKILL".into(),
        ));
    }
    Ok(())
}

#[cfg(debug_assertions)]
pub(crate) fn run_fence_ordering() -> std::result::Result<(), String> {
    fn run() -> Result<()> {
        let fixture = NativeSegmentFixture::new()?;
        let observer = fixture.observer()?;
        let start = observer.begin_observation(&fixture.expected)?;
        std::fs::write(fixture.temp.path().join("ordered"), b"ordered")?;
        let end = observer.end_fence(&fixture.expected, &start)?;
        let issued = observer.issued_fence(&fixture.expected, &end)?;
        let state = observer.shared.lock();
        let changed = state
            .events
            .iter()
            .find(|event| event.event.path.as_str() == "ordered")
            .ok_or_else(|| Error::Corrupt("ordered callback was not durably retained".into()))?;
        if changed.event.sequence >= end.sequence
            || changed.provider_event_id > issued.provider_event_id
            || changed.cut.durable_end_offset >= end.durable_offset
            || issued.durable_cut
                != state
                    .events
                    .iter()
                    .find(|event| event.event.sequence == end.sequence)
                    .ok_or_else(|| Error::Corrupt("durable fence marker was absent".into()))?
                    .cut
        {
            return Err(Error::Corrupt(
                "FlushSync fence was not ordered after durable callbacks".into(),
            ));
        }
        drop(state);
        let persisted: (i64, i64) = fixture.db.conn.query_row(
            "SELECT last_sequence,durable_end_offset
             FROM changed_path_observer_segments
             WHERE scope_id=?1 AND epoch=1 AND state='open'",
            [fixture.expected.scope_id.to_text()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if persisted.0 != i64::try_from(end.sequence).unwrap_or(-1)
            || persisted.1 != i64::try_from(end.durable_offset).unwrap_or(-1)
        {
            return Err(Error::Corrupt(
                "SegmentWriter did not persist the exact synchronous fence cut".into(),
            ));
        }
        Ok(())
    }
    run().map_err(|error| error.to_string())
}
