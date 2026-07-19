use std::collections::VecDeque;
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};

use super::codec::{encode_header, encode_record};
use super::*;
use crate::db::util::{apply_sqlite_runtime_pragmas, now_ts};

#[cfg(test)]
thread_local! {
    static APPEND_FLUSH_BOUNDARY_HOOK: std::cell::RefCell<
        Option<Box<dyn FnOnce(&Path, &Path)>>,
    > = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(super) fn install_append_flush_boundary_hook(hook: impl FnOnce(&Path, &Path) + 'static) {
    APPEND_FLUSH_BOUNDARY_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(test)]
fn run_append_flush_boundary_hook(workspace_db_dir: &Path, database_path: &Path) {
    APPEND_FLUSH_BOUNDARY_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook(workspace_db_dir, database_path);
        }
    });
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub(super) enum FaultPoint {
    AppendWrite,
    AppendPostWriteLeaseExpiry,
    FileSync,
    RotationOldSync,
    SealPublication,
    FirstDirectorySync,
    NextHeaderCreate,
    NextHeaderWrite,
    NextHeaderSync,
    SecondDirectorySync,
    NextMetadataPublication,
    Heartbeat,
}

#[derive(Debug, Default)]
pub(super) struct FaultScript {
    points: Mutex<VecDeque<FaultPoint>>,
    max_batch_capacity: Mutex<usize>,
}

impl FaultScript {
    #[cfg(test)]
    pub(super) fn new(points: impl IntoIterator<Item = FaultPoint>) -> Self {
        Self {
            points: Mutex::new(points.into_iter().collect()),
            max_batch_capacity: Mutex::new(0),
        }
    }

    fn check(&self, point: FaultPoint) -> std::io::Result<()> {
        if self.take(point) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                format!("injected observer I/O fault at {point:?}"),
            ));
        }
        Ok(())
    }

    fn take(&self, point: FaultPoint) -> bool {
        let mut points = self
            .points
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if points.front() == Some(&point) {
            points.pop_front();
            return true;
        }
        false
    }

    fn observe_batch_capacity(&self, capacity: usize) {
        let mut maximum = self
            .max_batch_capacity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *maximum = (*maximum).max(capacity);
    }

    #[cfg(test)]
    pub(super) fn max_batch_capacity(&self) -> usize {
        *self
            .max_batch_capacity
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

pub(crate) struct SegmentWriter {
    control: Connection,
    workspace_db_dir: PathBuf,
    database_path: PathBuf,
    segment_directory: PathBuf,
    file: File,
    pub(super) path: PathBuf,
    segment_id: String,
    previous_segment_id: Option<String>,
    identity: SegmentIdentity,
    provider_id: String,
    limits: PersistedLogLimits,
    lease_duration: Duration,
    authorized: bool,
    revoke_owner_on_drop: bool,
    current_offset: u64,
    /// Whether the currently open segment contains a record after its header.
    /// `last_sequence` is scope-global and intentionally survives rotation, so
    /// it cannot by itself distinguish a header-only segment.
    current_segment_has_records: bool,
    last_sequence: u64,
    last_hash: [u8; 32],
    last_cursor: Vec<u8>,
    faults: Arc<FaultScript>,
}

#[derive(Clone, Debug)]
pub(crate) struct DaemonLaunchBinding {
    pub(crate) nonce: String,
    pub(crate) pid: u32,
    pub(crate) process_start_identity: String,
}

impl SegmentWriter {
    pub(crate) fn bind_native_observer(
        &mut self,
        provider_identity: Vec<u8>,
        fence_nonce: Vec<u8>,
    ) -> Result<super::ObserverWriterBinding> {
        if provider_identity.is_empty() || fence_nonce.len() < 16 {
            return Err(Error::InvalidInput(
                "native observer binding requires provider identity and an unguessable fence nonce"
                    .into(),
            ));
        }
        if self.provider_id != hex::encode(&provider_identity) {
            return Err(Error::InvalidInput(
                "native observer provider identity does not match the acquired writer".into(),
            ));
        }
        let _workspace_lock = self.acquire_observer_publication_lock()?;
        self.ensure_authorized()?;
        let transaction = self
            .control
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_lease_on(&transaction, &self.identity)?;
        let owner_token = hex::encode(self.identity.owner_token);
        let provider_identity_text = hex::encode(&provider_identity);
        let owner_changed = transaction.execute(
            "UPDATE changed_path_observer_owners
             SET provider_identity=?1,fence_nonce=?2,updated_at=?3
             WHERE scope_id=?4 AND epoch=?5 AND owner_token=?6
               AND provider_id=?7 AND lease_state='active' AND expires_at>?3",
            params![
                provider_identity_text,
                fence_nonce,
                now_ts(),
                self.identity.scope_id.to_text(),
                sql_i64(self.identity.epoch, "observer epoch")?,
                owner_token,
                self.provider_id,
            ],
        )?;
        let scope_changed = transaction.execute(
            "UPDATE changed_path_scopes
             SET observer_owner_token=?1,updated_at=?2
             WHERE scope_id=?3 AND epoch=?4 AND provider_id=?5
               AND provider_identity=?6",
            params![
                owner_token,
                now_ts(),
                self.identity.scope_id.to_text(),
                sql_i64(self.identity.epoch, "observer epoch")?,
                self.provider_id,
                provider_identity_text,
            ],
        )?;
        if owner_changed != 1 || scope_changed != 1 {
            return Err(Error::WorkspaceLocked(
                "native observer binding lost its exact writer lease".into(),
            ));
        }
        transaction.commit()?;
        // A writer becomes runtime authority only after its native provider
        // identity and fence are durably bound. From this point onward an
        // ordinary runtime shutdown must revoke that exact owner and force a
        // full reconciliation before the scope can be trusted again.
        self.revoke_owner_on_drop = true;
        Ok(super::ObserverWriterBinding {
            owner_token,
            provider_id: self.provider_id.clone(),
            provider_identity,
            fence_nonce,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn acquire(
        database_path: &Path,
        segment_directory: &Path,
        scope_id: ScopeId,
        epoch: u64,
        owner_token: [u8; 32],
        provider_id: &str,
        provider_cursor: Vec<u8>,
        lease_duration: Duration,
    ) -> Result<Self> {
        Self::acquire_inner(
            database_path,
            segment_directory,
            scope_id,
            epoch,
            owner_token,
            provider_id,
            provider_cursor,
            lease_duration,
            None,
            Arc::new(FaultScript::default()),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn acquire_for_daemon(
        database_path: &Path,
        segment_directory: &Path,
        scope_id: ScopeId,
        epoch: u64,
        owner_token: [u8; 32],
        provider_id: &str,
        provider_cursor: Vec<u8>,
        lease_duration: Duration,
        daemon_launch: DaemonLaunchBinding,
    ) -> Result<Self> {
        Self::acquire_inner(
            database_path,
            segment_directory,
            scope_id,
            epoch,
            owner_token,
            provider_id,
            provider_cursor,
            lease_duration,
            Some(daemon_launch),
            Arc::new(FaultScript::default()),
        )
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(super) fn acquire_with_faults(
        database_path: &Path,
        segment_directory: &Path,
        scope_id: ScopeId,
        epoch: u64,
        owner_token: [u8; 32],
        provider_id: &str,
        provider_cursor: Vec<u8>,
        lease_duration: Duration,
        faults: Arc<FaultScript>,
    ) -> Result<Self> {
        Self::acquire_inner(
            database_path,
            segment_directory,
            scope_id,
            epoch,
            owner_token,
            provider_id,
            provider_cursor,
            lease_duration,
            None,
            faults,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn acquire_inner(
        database_path: &Path,
        segment_directory: &Path,
        scope_id: ScopeId,
        epoch: u64,
        owner_token: [u8; 32],
        provider_id: &str,
        provider_cursor: Vec<u8>,
        lease_duration: Duration,
        daemon_launch: Option<DaemonLaunchBinding>,
        faults: Arc<FaultScript>,
    ) -> Result<Self> {
        if epoch == 0 || provider_id.is_empty() || lease_duration.is_zero() {
            return Err(Error::InvalidInput(
                "observer lease requires positive epoch/duration and provider id".into(),
            ));
        }
        let workspace_db_dir = workspace_db_dir_for_database(database_path)?;
        let mut control = Connection::open(database_path).map_err(|error| {
            Error::DaemonUnavailable(format!(
                "observer segment writer could not open its control connection: {error}"
            ))
        })?;
        apply_sqlite_runtime_pragmas(&control).map_err(|error| {
            Error::DaemonUnavailable(format!(
                "observer segment writer could not configure its control connection: {error}"
            ))
        })?;
        control
            .busy_timeout(Duration::from_secs(5))
            .map_err(|error| {
                Error::DaemonUnavailable(format!(
                    "observer segment writer could not configure its busy timeout: {error}"
                ))
            })?;
        let scope_text = scope_id.to_text();
        let retry_command = control
            .query_row(
                "SELECT initialization.lane_name
                 FROM changed_path_scopes scope
                 JOIN lane_initializations initialization
                   ON initialization.lane_id=scope.owner_id
                 WHERE scope.scope_id=?1 AND scope.scope_kind='materialized_lane'",
                [&scope_text],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .ok()
            .flatten()
            .map(|lane| format!("trail lane repair-initialization {lane}"))
            .unwrap_or_else(|| "retry observer startup".to_string());
        let _workspace_lock = crate::db::acquire_workspace_lock_with_admission(
            &workspace_db_dir,
            database_path,
            crate::db::WorkspaceLockAdmission {
                purpose: crate::db::WorkspaceLockPurpose::ObserverStartup,
                operation_id: Some(&scope_text),
                deadline: crate::db::default_workspace_lock_admission_deadline()?,
                retry_command: &retry_command,
            },
        )?;
        let now = now_ts();
        let expires_at = lease_expiry(now, lease_duration)?;
        let epoch_sql = sql_i64(epoch, "observer epoch")?;
        let owner_text = hex::encode(owner_token);
        let transaction = control.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let limits = transaction
            .query_row(
                "SELECT max_observer_log_bytes, max_segment_bytes,
                            max_unfolded_tail_records
                     FROM changed_path_scopes WHERE scope_id = ?1 AND epoch = ?2",
                params![scope_text, epoch_sql],
                |row| {
                    Ok(PersistedLogLimits {
                        max_log_bytes: row.get::<_, i64>(0)?.try_into().map_err(|_| {
                            rusqlite::Error::IntegralValueOutOfRange(0, row.get(0).unwrap_or(-1))
                        })?,
                        max_segment_bytes: row.get::<_, i64>(1)?.try_into().map_err(|_| {
                            rusqlite::Error::IntegralValueOutOfRange(1, row.get(1).unwrap_or(-1))
                        })?,
                        max_unfolded_tail_records: row.get::<_, i64>(2)?.try_into().map_err(
                            |_| {
                                rusqlite::Error::IntegralValueOutOfRange(
                                    2,
                                    row.get(2).unwrap_or(-1),
                                )
                            },
                        )?,
                    })
                },
            )
            .optional()
            .map_err(|error| {
                Error::DaemonUnavailable(format!(
                    "observer segment writer could not read its persisted limits: {error}"
                ))
            })?
            .ok_or_else(|| Error::InvalidInput("observer scope/epoch is stale".into()))?;
        limits
            .validate()
            .map_err(|error| Error::Corrupt(error.to_string()))?;
        let existing_owner = transaction
            .query_row(
                "SELECT epoch, owner_token, lease_state, expires_at
                 FROM changed_path_observer_owners WHERE scope_id=?1",
                params![scope_text],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| {
                Error::DaemonUnavailable(format!(
                    "observer segment writer could not read its existing owner: {error}"
                ))
            })?;
        if let Some((owner_epoch, _token, state, owner_expiry)) = existing_owner {
            let owner_epoch = u64::try_from(owner_epoch)
                .map_err(|_| Error::Corrupt("negative observer owner epoch".into()))?;
            if owner_epoch == epoch {
                if state == "active" && owner_expiry > now {
                    return Err(Error::WorkspaceLocked(
                        "changed-path observer lease is already active".into(),
                    ));
                }
                return Err(Error::WorkspaceLocked(
                    "same-epoch observer owner replacement requires reconciliation and an authoritative epoch advance".into(),
                ));
            }
            if owner_epoch > epoch {
                return Err(Error::WorkspaceLocked(
                    "observer owner epoch is ahead of the requested epoch; reconciliation required"
                        .into(),
                ));
            }
        }
        let identity = SegmentIdentity {
            scope_id,
            epoch,
            owner_token,
            provider_cursor: provider_cursor.clone(),
            previous_segment_hash: [0; 32],
        };
        let header = encode_header(&identity).map_err(|error| Error::Corrupt(error.to_string()))?;
        let header_len = header.len() as u64;
        if header_len > limits.max_segment_bytes || header_len > limits.max_log_bytes {
            return Err(Error::InvalidInput(
                "observer segment header exceeds persisted byte cap".into(),
            ));
        }
        let segment_id = segment_id(epoch, 1, owner_token);
        let filename = segment_filename(&segment_id)?;
        let path = segment_directory.join(&filename);
        fs::create_dir_all(segment_directory)?;
        sync_directory(segment_directory)?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)?;
        super::super::secure_fs::lock_observer_writer(&file)?;
        file.write_all(&header)?;
        file.sync_all()?;
        sync_directory(segment_directory)?;
        let publish_now = now_ts();
        if publish_now >= expires_at {
            return Err(Error::WorkspaceLocked(
                "observer lease expired before initial publication".into(),
            ));
        }
        transaction.execute(
            "INSERT INTO changed_path_observer_owners(
                 scope_id, epoch, owner_token, provider_id, provider_identity,
                 lease_state, fence_nonce, acquired_at, heartbeat_at, expires_at,
                 error_state, error_at, updated_at
             ) VALUES(?1, ?2, ?3, ?4, '', 'active', NULL, ?5, ?5, ?6, NULL, NULL, ?5)
             ON CONFLICT(scope_id) DO UPDATE SET
                 epoch=excluded.epoch, owner_token=excluded.owner_token,
                 provider_id=excluded.provider_id, provider_identity=excluded.provider_identity,
                 lease_state='active', fence_nonce=NULL, acquired_at=excluded.acquired_at,
                 heartbeat_at=excluded.heartbeat_at, expires_at=excluded.expires_at,
                 error_state=NULL, error_at=NULL,
                 daemon_launch_nonce=NULL, daemon_pid=NULL,
                 daemon_process_start_identity=NULL,
                 updated_at=excluded.updated_at",
            params![
                scope_text,
                epoch_sql,
                owner_text,
                provider_id,
                now,
                expires_at
            ],
        )?;
        if let Some(daemon_launch) = &daemon_launch {
            if daemon_launch.nonce.len() != 64
                || daemon_launch.pid == 0
                || daemon_launch.process_start_identity.is_empty()
            {
                return Err(Error::InvalidInput(
                    "daemon observer launch binding is malformed".into(),
                ));
            }
            let bound = transaction.execute(
                "UPDATE changed_path_observer_owners
                 SET daemon_launch_nonce=?1,daemon_pid=?2,daemon_process_start_identity=?3
                 WHERE scope_id=?4 AND epoch=?5 AND owner_token=?6 AND lease_state='active'",
                params![
                    daemon_launch.nonce,
                    i64::from(daemon_launch.pid),
                    daemon_launch.process_start_identity,
                    scope_text,
                    epoch_sql,
                    owner_text,
                ],
            )?;
            if bound != 1 {
                return Err(Error::WorkspaceLocked(
                    "daemon observer owner launch binding lost exact authority".into(),
                ));
            }
        }
        let inserted = transaction.execute(
            "INSERT INTO changed_path_observer_segments(
                 scope_id, epoch, segment_id, log_format_version, owner_token,
                 provider_id, first_sequence, last_sequence, durable_end_offset,
                 folded_end_offset, previous_segment_id, previous_segment_hash,
                 segment_hash, segment_path, state, created_at, sealed_at, updated_at
             ) SELECT ?1, ?2, ?3, 1, ?4, ?5, 1, NULL, ?6, 0,
                      NULL, NULL, NULL, ?7, 'open', ?8, NULL, ?8
               WHERE EXISTS(
                   SELECT 1 FROM changed_path_observer_owners
                   WHERE scope_id=?1 AND epoch=?2 AND owner_token=?4
                     AND lease_state='active' AND expires_at>?8
               )",
            params![
                scope_text,
                epoch_sql,
                segment_id,
                owner_text,
                provider_id,
                sql_i64(header_len, "durable observer header offset")?,
                filename,
                publish_now
            ],
        )?;
        if inserted != 1 {
            return Err(Error::WorkspaceLocked(
                "observer lease changed before publication".into(),
            ));
        }
        transaction.commit()?;
        let current_offset = header_len;
        Ok(Self {
            control,
            workspace_db_dir,
            database_path: database_path.to_path_buf(),
            segment_directory: segment_directory.to_path_buf(),
            file,
            path,
            segment_id,
            previous_segment_id: None,
            identity,
            provider_id: provider_id.to_owned(),
            limits,
            lease_duration,
            authorized: true,
            revoke_owner_on_drop: daemon_launch.is_some(),
            current_offset,
            current_segment_has_records: false,
            last_sequence: 0,
            last_hash: [0; 32],
            last_cursor: provider_cursor,
            faults,
        })
    }

    pub(crate) fn append(&mut self, records: &[ObserverRecord]) -> Result<()> {
        let result = match self.acquire_observer_publication_lock() {
            Ok(_workspace_lock) => self.append_inner(records),
            Err(error) => Err(error),
        };
        if result.is_err() {
            self.retire("append_failed");
        }
        result
    }

    /// Append and publish one durable batch without exposing the
    /// file-before-SQLite window to a command workspace lock holder.
    pub(crate) fn append_and_flush(&mut self, records: &[ObserverRecord]) -> Result<DurableCut> {
        let result = (|| {
            let _workspace_lock = self.acquire_observer_publication_lock()?;
            self.append_inner(records)?;
            #[cfg(test)]
            run_append_flush_boundary_hook(&self.workspace_db_dir, &self.database_path);
            self.flush_inner()
        })();
        if result.is_err() {
            self.retire("append_and_flush_failed");
        }
        result
    }

    /// Append and publish an authenticated boundary record, then rotate that
    /// exact durable cut while retaining one observer workspace lock. The
    /// native durability worker calls this as one queue operation, so records
    /// ordered after the boundary cannot advance the segment before it seals.
    pub(crate) fn append_flush_and_rotate(
        &mut self,
        records: &[ObserverRecord],
    ) -> Result<(DurableCut, DurableCut)> {
        let result = (|| {
            let _workspace_lock = self.acquire_observer_publication_lock()?;
            self.append_inner(records)?;
            let sealed = self.flush_inner()?;
            self.rotate_inner()?;
            let anchor = DurableCut {
                segment_id: self.segment_id.clone(),
                durable_end_offset: self.current_offset,
                last_sequence: self.last_sequence,
                last_hash: self.last_hash,
                provider_cursor: self.last_cursor.clone(),
            };
            Ok((sealed, anchor))
        })();
        if result.is_err() {
            self.retire("append_flush_and_rotate_failed");
        }
        result
    }

    fn append_inner(&mut self, records: &[ObserverRecord]) -> Result<()> {
        self.ensure_authorized()?;
        let transaction = self
            .control
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_lease_on(&transaction, &self.identity)?;
        if records.is_empty() {
            transaction.commit()?;
            return Ok(());
        }
        let other_durable = transaction.query_row(
            "SELECT COALESCE(SUM(durable_end_offset), 0)
             FROM changed_path_observer_segments
             WHERE scope_id=?1 AND epoch=?2 AND segment_id<>?3",
            params![
                self.identity.scope_id.to_text(),
                sql_i64(self.identity.epoch, "observer epoch")?,
                self.segment_id,
            ],
            |row| row.get::<_, i64>(0),
        )?;
        let other_durable = u64::try_from(other_durable)
            .map_err(|_| Error::Corrupt("negative observer log byte total".into()))?;
        let segment_remaining = self
            .limits
            .max_segment_bytes
            .checked_sub(self.current_offset)
            .ok_or_else(|| Error::Corrupt("observer segment already exceeds byte cap".into()))?;
        let current_total = other_durable
            .checked_add(self.current_offset)
            .ok_or_else(|| Error::InvalidInput("observer log byte total overflow".into()))?;
        let log_remaining = self
            .limits
            .max_log_bytes
            .checked_sub(current_total)
            .ok_or_else(|| Error::Corrupt("observer log already exceeds byte cap".into()))?;
        let remaining = segment_remaining.min(log_remaining);
        let remaining_usize = usize::try_from(remaining).map_err(|_| {
            Error::InvalidInput("observer append capacity cannot fit memory".into())
        })?;
        let mut batch = Vec::new();
        let mut sequence = self.last_sequence;
        let mut hash = self.last_hash;
        for record in records {
            if record.sequence != sequence.saturating_add(1) {
                return Err(Error::InvalidInput(
                    "observer append sequence is not exactly monotonic".into(),
                ));
            }
            let (encoded, next_hash) = encode_record(record, hash)
                .map_err(|error| Error::InvalidInput(error.to_string()))?;
            let next_batch_len = batch
                .len()
                .checked_add(encoded.len())
                .ok_or_else(|| Error::InvalidInput("observer append batch overflow".into()))?;
            if next_batch_len > remaining_usize {
                return Err(Error::InvalidInput(
                    "observer append batch exceeds persisted remaining byte cap".into(),
                ));
            }
            batch.try_reserve_exact(encoded.len()).map_err(|_| {
                Error::InvalidInput("observer append batch allocation failed".into())
            })?;
            if batch.capacity() > remaining_usize {
                return Err(Error::InvalidInput(
                    "observer append batch allocation exceeds persisted remaining byte cap".into(),
                ));
            }
            batch.extend_from_slice(&encoded);
            self.faults.observe_batch_capacity(batch.capacity());
            sequence = record.sequence;
            hash = next_hash;
        }
        let next_offset = self
            .current_offset
            .checked_add(batch.len() as u64)
            .ok_or_else(|| Error::InvalidInput("observer segment offset overflow".into()))?;
        validate_lease_on(&transaction, &self.identity)?;
        self.faults.check(FaultPoint::AppendWrite)?;
        self.file.write_all(&batch)?;
        if self.faults.take(FaultPoint::AppendPostWriteLeaseExpiry) {
            transaction.execute(
                "UPDATE changed_path_observer_owners
                 SET expires_at=heartbeat_at
                 WHERE scope_id=?1 AND epoch=?2 AND owner_token=?3",
                params![
                    self.identity.scope_id.to_text(),
                    sql_i64(self.identity.epoch, "observer epoch")?,
                    hex::encode(self.identity.owner_token)
                ],
            )?;
        }
        validate_lease_on(&transaction, &self.identity)?;
        transaction.commit()?;
        self.current_offset = next_offset;
        self.current_segment_has_records = true;
        self.last_sequence = sequence;
        self.last_hash = hash;
        self.last_cursor = records.last().unwrap().provider_cursor.clone();
        Ok(())
    }

    pub(crate) fn flush_durable(&mut self) -> Result<DurableCut> {
        let result = match self.acquire_observer_publication_lock() {
            Ok(_workspace_lock) => self.flush_inner(),
            Err(error) => Err(error),
        };
        if result.is_err() {
            self.retire("flush_failed");
        }
        result
    }

    fn flush_inner(&mut self) -> Result<DurableCut> {
        self.ensure_authorized()?;
        let transaction = self
            .control
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_lease_on(&transaction, &self.identity)?;
        self.faults.check(FaultPoint::FileSync)?;
        self.file.sync_data()?;
        if self.file.metadata()?.len() != self.current_offset {
            return Err(Error::Corrupt(
                "observer segment length differs from the claimed durable offset".into(),
            ));
        }
        validate_lease_on(&transaction, &self.identity)?;
        let changed = transaction.execute(
            "UPDATE changed_path_observer_segments
             SET durable_end_offset=?1, last_sequence=?2, updated_at=?3
             WHERE scope_id=?4 AND epoch=?5 AND segment_id=?6 AND owner_token=?7
               AND state='open'
               AND EXISTS(
                   SELECT 1 FROM changed_path_observer_owners
                   WHERE scope_id=?4 AND epoch=?5 AND owner_token=?7
                     AND lease_state='active' AND expires_at>?3
               )",
            params![
                sql_i64(self.current_offset, "durable observer offset")?,
                if !self.current_segment_has_records {
                    None
                } else {
                    Some(sql_i64(self.last_sequence, "observer sequence")?)
                },
                now_ts(),
                self.identity.scope_id.to_text(),
                sql_i64(self.identity.epoch, "observer epoch")?,
                self.segment_id,
                hex::encode(self.identity.owner_token),
            ],
        )?;
        if changed != 1 {
            return Err(Error::WorkspaceLocked(
                "observer lease changed before durable publication".into(),
            ));
        }
        transaction.commit()?;
        Ok(DurableCut {
            segment_id: self.segment_id.clone(),
            durable_end_offset: self.current_offset,
            last_sequence: self.last_sequence,
            last_hash: self.last_hash,
            provider_cursor: self.last_cursor.clone(),
        })
    }

    pub(crate) fn heartbeat(&mut self) -> Result<()> {
        // Lease renewal publishes no filesystem evidence and changes no
        // segment boundary. It must remain live while an ordinary command
        // holds the high-level workspace lock for a long materialization or
        // checkpoint. SQLite serialization plus the exact owner CAS below are
        // the authority boundary for this owner-only heartbeat row.
        let result = self.heartbeat_inner();
        if result.is_err() {
            self.retire("heartbeat_failed");
        }
        result
    }

    /// Durably revokes the exact active observer owner after its terminal
    /// invalidation marker has been flushed.
    pub(crate) fn revoke(&mut self, reason: &str) -> Result<()> {
        self.ensure_authorized()?;
        let _workspace_lock = self.acquire_observer_publication_lock()?;
        let transaction = self
            .control
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_lease_on(&transaction, &self.identity)?;
        let changed = transaction.execute(
            "UPDATE changed_path_observer_owners
             SET lease_state='error', error_state=?1, error_at=?2, updated_at=?2,
                 expires_at=?2
             WHERE scope_id=?3 AND epoch=?4 AND owner_token=?5
               AND lease_state='active'",
            params![
                reason,
                now_ts(),
                self.identity.scope_id.to_text(),
                sql_i64(self.identity.epoch, "observer epoch")?,
                hex::encode(self.identity.owner_token),
            ],
        )?;
        if changed != 1 {
            return Err(Error::WorkspaceLocked(
                "observer owner changed before durable revocation".into(),
            ));
        }
        transaction.commit()?;
        self.authorized = false;
        Ok(())
    }

    fn heartbeat_inner(&mut self) -> Result<()> {
        self.validate_lease()?;
        self.faults.check(FaultPoint::Heartbeat)?;
        let now = now_ts();
        let expiry = lease_expiry(now, self.lease_duration)?;
        let changed = self.control.execute(
            "UPDATE changed_path_observer_owners
             SET heartbeat_at=?1, expires_at=?2, updated_at=?1
             WHERE scope_id=?3 AND epoch=?4 AND owner_token=?5
               AND lease_state='active' AND expires_at>?1",
            params![
                now,
                expiry,
                self.identity.scope_id.to_text(),
                sql_i64(self.identity.epoch, "observer epoch")?,
                hex::encode(self.identity.owner_token)
            ],
        )?;
        if changed != 1 {
            return Err(Error::WorkspaceLocked(
                "observer heartbeat lost its lease".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn rotate(&mut self) -> Result<()> {
        self.rotate_and_cut().map(|_| ())
    }

    /// Seal the current append-only segment and return the exact durable cut
    /// authenticated by that segment.  Controlled filesystem producers use
    /// this to prove that an intent interval starts and ends on unambiguous
    /// sidecar boundaries.
    pub(crate) fn rotate_and_cut(&mut self) -> Result<DurableCut> {
        self.rotate_and_cuts().map(|(sealed, _anchor)| sealed)
    }

    pub(crate) fn rotate_and_cuts(&mut self) -> Result<(DurableCut, DurableCut)> {
        let sealed = DurableCut {
            segment_id: self.segment_id.clone(),
            durable_end_offset: self.current_offset,
            last_sequence: self.last_sequence,
            last_hash: self.last_hash,
            provider_cursor: self.last_cursor.clone(),
        };
        let result = match self.acquire_observer_publication_lock() {
            Ok(_workspace_lock) => self.rotate_inner().map(|_| {
                let anchor = DurableCut {
                    segment_id: self.segment_id.clone(),
                    durable_end_offset: self.current_offset,
                    last_sequence: self.last_sequence,
                    last_hash: self.last_hash,
                    provider_cursor: self.last_cursor.clone(),
                };
                (sealed, anchor)
            }),
            Err(error) => Err(error),
        };
        if result.is_err() {
            self.retire("rotation_failed");
        }
        result
    }

    pub(crate) fn rotate_if_cut(
        &mut self,
        expected: &DurableCut,
    ) -> Result<Option<(DurableCut, DurableCut)>> {
        let current = DurableCut {
            segment_id: self.segment_id.clone(),
            durable_end_offset: self.current_offset,
            last_sequence: self.last_sequence,
            last_hash: self.last_hash,
            provider_cursor: self.last_cursor.clone(),
        };
        if &current != expected {
            return Ok(None);
        }
        self.rotate_and_cuts().map(Some)
    }

    fn rotate_inner(&mut self) -> Result<()> {
        self.ensure_authorized()?;
        let transaction = self
            .control
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        validate_lease_on(&transaction, &self.identity)?;
        self.faults.check(FaultPoint::RotationOldSync)?;
        self.file.sync_data()?;
        let bytes = fs::read(&self.path)?;
        if bytes.len() as u64 != self.current_offset {
            return Err(Error::Corrupt(
                "observer segment length changed during rotation".into(),
            ));
        }
        // A header-only segment already represents an exact durable boundary.
        // Rotating it cannot advance `first_sequence`, and both the segment-id
        // derivation and schema intentionally reject a second segment for the
        // same first sequence. Keep the existing open anchor after rechecking
        // its durability and owner authority.
        if !self.current_segment_has_records {
            validate_lease_on(&transaction, &self.identity)?;
            transaction.commit()?;
            return Ok(());
        }
        let old_segment_hash: [u8; 32] = Sha256::digest(&bytes).into();
        self.faults.check(FaultPoint::FirstDirectorySync)?;
        sync_directory(&self.segment_directory)?;

        let first_sequence = self.last_sequence.saturating_add(1).max(1);
        let next_segment_id = segment_id(
            self.identity.epoch,
            first_sequence,
            self.identity.owner_token,
        );
        let next_path = self
            .segment_directory
            .join(segment_filename(&next_segment_id)?);
        let next_identity = SegmentIdentity {
            scope_id: self.identity.scope_id,
            epoch: self.identity.epoch,
            owner_token: self.identity.owner_token,
            provider_cursor: self.last_cursor.clone(),
            previous_segment_hash: old_segment_hash,
        };
        let header =
            encode_header(&next_identity).map_err(|error| Error::Corrupt(error.to_string()))?;
        let header_len = header.len() as u64;
        if header_len > self.limits.max_segment_bytes {
            return Err(Error::InvalidInput(
                "observer rotation header exceeds segment byte cap".into(),
            ));
        }
        let other_durable = transaction.query_row(
            "SELECT COALESCE(SUM(durable_end_offset), 0)
             FROM changed_path_observer_segments
             WHERE scope_id=?1 AND epoch=?2 AND segment_id<>?3",
            params![
                self.identity.scope_id.to_text(),
                sql_i64(self.identity.epoch, "observer epoch")?,
                self.segment_id,
            ],
            |row| row.get::<_, i64>(0),
        )?;
        let total_after_rotation = u64::try_from(other_durable)
            .map_err(|_| Error::Corrupt("negative observer log byte total".into()))?
            .checked_add(self.current_offset)
            .and_then(|total| total.checked_add(header_len))
            .ok_or_else(|| Error::InvalidInput("observer log byte total overflow".into()))?;
        if total_after_rotation > self.limits.max_log_bytes {
            return Err(Error::InvalidInput(
                "observer rotation header exceeds total log byte cap".into(),
            ));
        }
        self.faults.check(FaultPoint::NextHeaderCreate)?;
        let mut next_file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&next_path)?;
        super::super::secure_fs::lock_observer_writer(&next_file)?;
        self.faults.check(FaultPoint::NextHeaderWrite)?;
        next_file.write_all(&header)?;
        self.faults.check(FaultPoint::NextHeaderSync)?;
        next_file.sync_all()?;
        self.faults.check(FaultPoint::SecondDirectorySync)?;
        sync_directory(&self.segment_directory)?;
        validate_lease_on(&transaction, &self.identity)?;
        let now = now_ts();
        self.faults.check(FaultPoint::SealPublication)?;
        let sealed = transaction.execute(
            "UPDATE changed_path_observer_segments
             SET state='sealed', last_sequence=?1, durable_end_offset=?2,
                 segment_hash=?3, sealed_at=?4, updated_at=?4
             WHERE scope_id=?5 AND epoch=?6 AND segment_id=?7 AND owner_token=?8
               AND state='open'
               AND EXISTS(
                   SELECT 1 FROM changed_path_observer_owners
                   WHERE scope_id=?5 AND epoch=?6 AND owner_token=?8
                     AND lease_state='active' AND expires_at>?4
               )",
            params![
                if !self.current_segment_has_records {
                    None
                } else {
                    Some(sql_i64(self.last_sequence, "observer sequence")?)
                },
                sql_i64(self.current_offset, "durable observer offset")?,
                hex::encode(old_segment_hash),
                now,
                self.identity.scope_id.to_text(),
                sql_i64(self.identity.epoch, "observer epoch")?,
                self.segment_id,
                hex::encode(self.identity.owner_token),
            ],
        )?;
        if sealed != 1 {
            return Err(Error::WorkspaceLocked(
                "observer lease changed before sealing".into(),
            ));
        }
        self.faults.check(FaultPoint::NextMetadataPublication)?;
        let next_filename = segment_filename(&next_segment_id)?;
        let published = transaction.execute(
            "INSERT INTO changed_path_observer_segments(
                 scope_id, epoch, segment_id, log_format_version, owner_token,
                 provider_id, first_sequence, last_sequence, durable_end_offset,
                 folded_end_offset, previous_segment_id, previous_segment_hash,
                 segment_hash, segment_path, state, created_at, sealed_at, updated_at
             ) SELECT ?1, ?2, ?3, 1, ?4, ?5, ?6, NULL, ?7, 0,
                      ?8, ?9, NULL, ?10, 'open', ?11, NULL, ?11
               WHERE EXISTS(
                   SELECT 1 FROM changed_path_observer_owners
                   WHERE scope_id=?1 AND epoch=?2 AND owner_token=?4
                     AND lease_state='active' AND expires_at>?11
               )",
            params![
                self.identity.scope_id.to_text(),
                sql_i64(self.identity.epoch, "observer epoch")?,
                next_segment_id,
                hex::encode(self.identity.owner_token),
                self.provider_id,
                sql_i64(first_sequence, "observer sequence")?,
                sql_i64(header_len, "durable observer header offset")?,
                self.segment_id,
                hex::encode(old_segment_hash),
                next_filename,
                now,
            ],
        )?;
        if published != 1 {
            return Err(Error::WorkspaceLocked(
                "observer lease changed before rotation publication".into(),
            ));
        }
        transaction.commit()?;
        self.file = next_file;
        self.path = next_path;
        self.previous_segment_id = Some(self.segment_id.clone());
        self.segment_id = next_segment_id;
        self.identity = next_identity;
        self.current_offset = header_len;
        self.current_segment_has_records = false;
        self.last_hash = [0; 32];
        Ok(())
    }

    fn ensure_authorized(&self) -> Result<()> {
        if !self.authorized {
            return Err(Error::WorkspaceLocked("observer writer is retired".into()));
        }
        Ok(())
    }

    fn validate_lease(&self) -> Result<()> {
        self.ensure_authorized()?;
        validate_lease_on(&self.control, &self.identity)
    }

    fn acquire_observer_publication_lock(&self) -> Result<crate::db::WorkspaceLock> {
        let operation_id = self.identity.scope_id.to_text();
        let workspace_lock = crate::db::acquire_workspace_lock_for_observer(
            &self.workspace_db_dir,
            &self.database_path,
            &operation_id,
        )?;
        // A same-process command convoy can intentionally hold observer
        // publication behind authorized write exclusion for longer than one
        // lease interval. The retained SegmentWriter still owns the locked
        // sidecar and exact active owner token. Refresh that same capability
        // after admission, before publishing any new segment boundary; an
        // epoch/token/state change remains a terminal CAS failure.
        let now = now_ts();
        let expiry = lease_expiry(now, self.lease_duration)?;
        let changed = self.control.execute(
            "UPDATE changed_path_observer_owners
             SET heartbeat_at=?1, expires_at=?2, updated_at=?1
             WHERE scope_id=?3 AND epoch=?4 AND owner_token=?5
               AND lease_state='active'",
            params![
                now,
                expiry,
                self.identity.scope_id.to_text(),
                sql_i64(self.identity.epoch, "observer epoch")?,
                hex::encode(self.identity.owner_token)
            ],
        )?;
        if changed != 1 {
            return Err(Error::WorkspaceLocked(
                "observer owner changed while publication was waiting".into(),
            ));
        }
        Ok(workspace_lock)
    }

    fn retire(&mut self, reason: &str) {
        if self.authorized {
            if let Ok(_workspace_lock) = self.acquire_observer_publication_lock() {
                revoke_owner(
                    &self.control,
                    &self.identity.scope_id.to_text(),
                    self.identity.epoch,
                    &hex::encode(self.identity.owner_token),
                    reason,
                );
            }
            self.authorized = false;
        }
    }

    #[cfg(test)]
    pub(super) fn is_authorized(&self) -> bool {
        self.authorized
    }

    #[cfg(test)]
    pub(super) fn runtime_pragmas(&self) -> (String, i64, i64, i64) {
        (
            self.control
                .query_row("PRAGMA journal_mode", [], |row| row.get(0))
                .unwrap(),
            self.control
                .query_row("PRAGMA foreign_keys", [], |row| row.get(0))
                .unwrap(),
            self.control
                .query_row("PRAGMA synchronous", [], |row| row.get(0))
                .unwrap(),
            self.control
                .query_row("PRAGMA temp_store", [], |row| row.get(0))
                .unwrap(),
        )
    }
}

impl Drop for SegmentWriter {
    fn drop(&mut self) {
        if self.revoke_owner_on_drop {
            self.retire("observer_writer_dropped");
        }
    }
}

pub(crate) fn workspace_db_dir_for_database(database_path: &Path) -> Result<PathBuf> {
    let parent = database_path.parent().ok_or_else(|| {
        Error::InvalidInput("observer database path has no workspace directory".into())
    })?;
    if parent.file_name().is_some_and(|name| name == "index") {
        return parent.parent().map(Path::to_path_buf).ok_or_else(|| {
            Error::InvalidInput("observer database index has no workspace directory".into())
        });
    }
    Ok(parent.to_path_buf())
}

fn validate_lease_on(connection: &Connection, identity: &SegmentIdentity) -> Result<()> {
    let exists = connection.query_row(
        "SELECT EXISTS(
                 SELECT 1 FROM changed_path_observer_owners owner
                 JOIN changed_path_scopes scope ON scope.scope_id=owner.scope_id
                 WHERE owner.scope_id=?1 AND owner.epoch=?2 AND scope.epoch=?2
                   AND owner.owner_token=?3 AND owner.lease_state='active'
                   AND owner.expires_at>?4
            )",
        params![
            identity.scope_id.to_text(),
            sql_i64(identity.epoch, "observer epoch")?,
            hex::encode(identity.owner_token),
            now_ts()
        ],
        |row| row.get::<_, bool>(0),
    )?;
    if !exists {
        return Err(Error::WorkspaceLocked(
            "observer lease is stale or expired".into(),
        ));
    }
    Ok(())
}

fn lease_expiry(now: i64, duration: Duration) -> Result<i64> {
    let seconds = i64::try_from(duration.as_secs().max(1))
        .map_err(|_| Error::InvalidInput("observer lease duration exceeds range".into()))?;
    now.checked_add(seconds)
        .ok_or_else(|| Error::InvalidInput("observer lease expiry overflow".into()))
}

pub(super) fn segment_id(epoch: u64, first_sequence: u64, owner_token: [u8; 32]) -> String {
    format!(
        "{epoch:020}-{first_sequence:020}-{}",
        hex::encode(owner_token)
    )
}

pub(super) fn segment_filename(segment_id: &str) -> Result<String> {
    let bytes = segment_id.as_bytes();
    let epoch = std::str::from_utf8(bytes.get(..20).unwrap_or_default())
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    let first_sequence = std::str::from_utf8(bytes.get(21..41).unwrap_or_default())
        .ok()
        .and_then(|value| value.parse::<u64>().ok());
    let valid = bytes.len() == 106
        && bytes[..20].iter().all(u8::is_ascii_digit)
        && bytes[20] == b'-'
        && bytes[21..41].iter().all(u8::is_ascii_digit)
        && bytes[41] == b'-'
        && bytes[42..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        && epoch.is_some_and(|epoch| epoch > 0 && format!("{epoch:020}") == segment_id[..20])
        && first_sequence.is_some_and(|sequence| {
            sequence > 0 && format!("{sequence:020}") == segment_id[21..41]
        });
    if !valid {
        return Err(Error::Corrupt("invalid observer segment id".into()));
    }
    let filename = format!("{segment_id}.cpl");
    if filename.len() > MAX_SEGMENT_FILENAME_BYTES {
        return Err(Error::Corrupt(
            "observer segment filename exceeds bound".into(),
        ));
    }
    Ok(filename)
}

pub(super) fn sync_directory(path: &Path) -> std::io::Result<()> {
    File::open(path)?.sync_all()
}

fn revoke_owner(
    connection: &Connection,
    scope_id: &str,
    epoch: u64,
    owner_token: &str,
    reason: &str,
) {
    let now = now_ts();
    let Ok(epoch) = i64::try_from(epoch) else {
        return;
    };
    let _ = connection.execute(
        "UPDATE changed_path_observer_owners
         SET lease_state='error', error_state=?1, error_at=?2, updated_at=?2
         WHERE scope_id=?3 AND epoch=?4 AND owner_token=?5 AND lease_state='active'",
        params![reason, now, scope_id, epoch, owner_token],
    );
}
