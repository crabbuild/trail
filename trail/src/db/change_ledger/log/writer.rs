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
use crate::db::util::{apply_sqlite_pragmas, now_ts};

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
}

impl FaultScript {
    #[cfg(test)]
    pub(super) fn new(points: impl IntoIterator<Item = FaultPoint>) -> Self {
        Self {
            points: Mutex::new(points.into_iter().collect()),
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
}

pub(crate) struct SegmentWriter {
    control: Connection,
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
    current_offset: u64,
    last_sequence: u64,
    last_hash: [u8; 32],
    last_cursor: Vec<u8>,
    faults: Arc<FaultScript>,
}

impl SegmentWriter {
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
        faults: Arc<FaultScript>,
    ) -> Result<Self> {
        if epoch == 0 || provider_id.is_empty() || lease_duration.is_zero() {
            return Err(Error::InvalidInput(
                "observer lease requires positive epoch/duration and provider id".into(),
            ));
        }
        let mut control = Connection::open(database_path)?;
        apply_sqlite_pragmas(&control)?;
        control.busy_timeout(Duration::from_secs(5))?;
        let now = now_ts();
        let expires_at = lease_expiry(now, lease_duration)?;
        let scope_text = scope_id.to_text();
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
            .optional()?
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
            .optional()?;
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
        let segment_id = segment_id(1, owner_token);
        let filename = segment_filename(&segment_id)?;
        let path = segment_directory.join(&filename);
        fs::create_dir_all(segment_directory)?;
        sync_directory(segment_directory)?;
        let mut file = OpenOptions::new()
            .create_new(true)
            .read(true)
            .write(true)
            .open(&path)?;
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
                 error_state=NULL, error_at=NULL, updated_at=excluded.updated_at",
            params![
                scope_text,
                epoch_sql,
                owner_text,
                provider_id,
                now,
                expires_at
            ],
        )?;
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
            current_offset,
            last_sequence: 0,
            last_hash: [0; 32],
            last_cursor: provider_cursor,
            faults,
        })
    }

    pub(crate) fn append(&mut self, records: &[ObserverRecord]) -> Result<()> {
        let result = self.append_inner(records);
        if result.is_err() {
            self.retire("append_failed");
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
            batch.extend_from_slice(&encoded);
            sequence = record.sequence;
            hash = next_hash;
        }
        let next_offset = self
            .current_offset
            .checked_add(batch.len() as u64)
            .ok_or_else(|| Error::InvalidInput("observer segment offset overflow".into()))?;
        if next_offset > self.limits.max_segment_bytes {
            return Err(Error::InvalidInput(
                "observer segment byte cap exceeded".into(),
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
        let other_durable = u64::try_from(other_durable)
            .map_err(|_| Error::Corrupt("negative observer log byte total".into()))?;
        let total_bytes = other_durable
            .checked_add(next_offset)
            .ok_or_else(|| Error::InvalidInput("observer log byte total overflow".into()))?;
        if total_bytes > self.limits.max_log_bytes {
            return Err(Error::InvalidInput("observer log byte cap exceeded".into()));
        }
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
        self.last_sequence = sequence;
        self.last_hash = hash;
        self.last_cursor = records.last().unwrap().provider_cursor.clone();
        Ok(())
    }

    pub(crate) fn flush_durable(&mut self) -> Result<DurableCut> {
        let result = self.flush_inner();
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
                if self.last_sequence == 0 {
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
        let result = self.heartbeat_inner();
        if result.is_err() {
            self.retire("heartbeat_failed");
        }
        result
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
        let result = self.rotate_inner();
        if result.is_err() {
            self.retire("rotation_failed");
        }
        result
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
        let old_segment_hash: [u8; 32] = Sha256::digest(&bytes).into();
        self.faults.check(FaultPoint::FirstDirectorySync)?;
        sync_directory(&self.segment_directory)?;

        let first_sequence = self.last_sequence.saturating_add(1).max(1);
        let next_segment_id = segment_id(first_sequence, self.identity.owner_token);
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
                if self.last_sequence == 0 {
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

    fn retire(&mut self, reason: &str) {
        if self.authorized {
            self.authorized = false;
            revoke_owner(
                &self.control,
                &self.identity.scope_id.to_text(),
                self.identity.epoch,
                &hex::encode(self.identity.owner_token),
                reason,
            );
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

fn segment_id(first_sequence: u64, owner_token: [u8; 32]) -> String {
    format!("{first_sequence:020}-{}", &hex::encode(owner_token)[..16])
}

pub(super) fn segment_filename(segment_id: &str) -> Result<String> {
    let bytes = segment_id.as_bytes();
    let valid = bytes.len() == 37
        && bytes[..20].iter().all(u8::is_ascii_digit)
        && bytes[20] == b'-'
        && bytes[21..]
            .iter()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte));
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
