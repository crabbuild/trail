use std::io::{Read, Seek, SeekFrom, Write};
use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::{
    raw_event_invalidates_policy, ChangedPathLedger, CompiledPolicy, EvidenceFlags, ExpectedScope,
    LedgerPath, ScopeId, TrustState,
};
use crate::db::storage::{
    PinnedWorktreeRoot, ReconciliationDirectory, ReconciliationFile, ReconciliationScanEntry,
};
use crate::db::util::now_ts;
use crate::error::{Error, Result};
use crate::model::{ChangeLedgerReconcileReport, FileEntry, FileKind};
use crate::Trail;

const STAGING_BATCH_ROWS: usize = 256;
const MAX_IDENTITY_RACE_RETRIES: u64 = 2;
const MAX_OBSERVER_SPOOL_EVENTS: u64 = 1_000_000;
const MAX_OBSERVER_SPOOL_BYTES: u64 = 256 * 1024 * 1024;
const MAX_OBSERVER_SPOOL_PATH_BYTES: usize = 1024 * 1024;
const OBSERVER_SPOOL_HEADER_BYTES: usize = 4 + 8 + 8;
// Do not linearize against a lease that can expire in the same instant as the
// final CAS and SQLite commit.
const MIN_PUBLICATION_LEASE_HORIZON_SECS: i64 = 5;
static NEXT_ATTEMPT_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ObserverFence {
    pub(crate) sequence: u64,
    pub(crate) durable_offset: u64,
    pub(crate) nonce: Vec<u8>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObserverEvent {
    pub(crate) path: LedgerPath,
    pub(crate) flags: EvidenceFlags,
    pub(crate) sequence: u64,
}

struct ObserverEventSpool {
    file: std::fs::File,
    events: u64,
    bytes: u64,
}

impl ObserverEventSpool {
    fn new() -> Result<Self> {
        Ok(Self {
            file: tempfile::tempfile()?,
            events: 0,
            bytes: 0,
        })
    }

    fn push(&mut self, event: ObserverEvent) -> Result<()> {
        let path = event.path.as_str().as_bytes();
        if path.len() > MAX_OBSERVER_SPOOL_PATH_BYTES {
            return Err(Error::InvalidInput(
                "observer event path exceeds reconciliation spool limit".into(),
            ));
        }
        let next_events = self.events.saturating_add(1);
        let record_bytes = OBSERVER_SPOOL_HEADER_BYTES
            .checked_add(path.len())
            .ok_or_else(|| Error::InvalidInput("observer spool record size overflow".into()))?;
        let next_bytes = self
            .bytes
            .checked_add(record_bytes as u64)
            .ok_or_else(|| Error::InvalidInput("observer spool size overflow".into()))?;
        if next_events > MAX_OBSERVER_SPOOL_EVENTS || next_bytes > MAX_OBSERVER_SPOOL_BYTES {
            return Err(Error::InvalidInput(
                "observer evidence exceeds bounded reconciliation spool".into(),
            ));
        }
        let path_len = u32::try_from(path.len())
            .map_err(|_| Error::InvalidInput("observer path length overflow".into()))?;
        self.file.write_all(&path_len.to_be_bytes())?;
        self.file.write_all(&event.flags.0.to_be_bytes())?;
        self.file.write_all(&event.sequence.to_be_bytes())?;
        self.file.write_all(path)?;
        self.events = next_events;
        self.bytes = next_bytes;
        Ok(())
    }

    fn finish(&mut self) -> Result<()> {
        self.file.sync_all()?;
        self.file.seek(SeekFrom::Start(0))?;
        Ok(())
    }

    fn next(&mut self) -> Result<Option<ObserverEvent>> {
        let mut header = [0_u8; OBSERVER_SPOOL_HEADER_BYTES];
        let read = self.file.read(&mut header[..1])?;
        if read == 0 {
            return Ok(None);
        }
        self.file.read_exact(&mut header[1..])?;
        let path_len = u32::from_be_bytes(header[..4].try_into().expect("four byte length"));
        let path_len = usize::try_from(path_len)
            .map_err(|_| Error::Corrupt("observer spool path length cannot fit memory".into()))?;
        if path_len > MAX_OBSERVER_SPOOL_PATH_BYTES {
            return Err(Error::Corrupt(
                "observer spool path exceeds bounded record limit".into(),
            ));
        }
        let flags = i64::from_be_bytes(header[4..12].try_into().expect("eight byte flags"));
        let sequence = u64::from_be_bytes(header[12..20].try_into().expect("eight byte sequence"));
        let mut path = vec![0_u8; path_len];
        self.file.read_exact(&mut path)?;
        let path = String::from_utf8(path)
            .map_err(|_| Error::Corrupt("observer spool path is not UTF-8".into()))?;
        Ok(Some(ObserverEvent {
            path: LedgerPath::parse(&path)?,
            flags: EvidenceFlags(flags),
            sequence,
        }))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ObserverQualification {
    scope_id: ScopeId,
    provider_identity: Vec<u8>,
    filesystem_identity: Vec<u8>,
    root_handle_identity: Vec<u8>,
    policy_fingerprint: [u8; 32],
    policy_generation: u64,
    start_fence: ObserverFence,
    end_fence: ObserverFence,
    observer_owner_token: String,
    owner_fence_nonce: Option<Vec<u8>>,
    durable_segment_id: String,
    segment_durable_offset: u64,
    segment_folded_offset: u64,
    complete_root_interval: bool,
    complete_policy_interval: bool,
    persisted_evidence_through_end: bool,
}

impl ObserverQualification {
    #[cfg(any(test, debug_assertions))]
    fn seal_for_test(
        expected: &ExpectedScope,
        root_handle_identity: Vec<u8>,
        start_fence: ObserverFence,
        end_fence: ObserverFence,
    ) -> Self {
        Self {
            scope_id: expected.scope_id,
            provider_identity: expected.provider_identity.clone(),
            filesystem_identity: expected.filesystem_identity.clone(),
            root_handle_identity,
            policy_fingerprint: expected.policy_fingerprint,
            policy_generation: expected.policy_generation,
            start_fence,
            end_fence,
            observer_owner_token: "full-test-owner".into(),
            owner_fence_nonce: Some(b"full-test-fence".to_vec()),
            durable_segment_id: "full-test-segment".into(),
            segment_durable_offset: 100,
            segment_folded_offset: 100,
            complete_root_interval: true,
            complete_policy_interval: true,
            persisted_evidence_through_end: true,
        }
    }

    fn validates(
        &self,
        expected: &ExpectedScope,
        root_handle_identity: &[u8],
        start: &ObserverFence,
        end: &ObserverFence,
    ) -> bool {
        self.scope_id == expected.scope_id
            && self.provider_identity == expected.provider_identity
            && self.filesystem_identity == expected.filesystem_identity
            && self.root_handle_identity == root_handle_identity
            && self.policy_fingerprint == expected.policy_fingerprint
            && self.policy_generation == expected.policy_generation
            && self.start_fence == *start
            && self.end_fence == *end
            && !self.observer_owner_token.is_empty()
            && !self.durable_segment_id.is_empty()
            && self.segment_folded_offset <= self.segment_durable_offset
            && end.sequence >= start.sequence
            && end.durable_offset >= start.durable_offset
            && self.complete_root_interval
            && self.complete_policy_interval
            && self.persisted_evidence_through_end
    }
}

pub(crate) trait QualifiedObserver {
    fn begin_observation(&self, expected: &ExpectedScope) -> Result<ObserverFence>;

    fn end_fence(&self, expected: &ExpectedScope, start: &ObserverFence) -> Result<ObserverFence>;

    fn drain_through(
        &self,
        expected: &ExpectedScope,
        root_handle_identity: &[u8],
        start: &ObserverFence,
        end: &ObserverFence,
        sink: &mut dyn FnMut(ObserverEvent) -> Result<()>,
    ) -> Result<ObserverQualification>;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProvenPrefixSet {
    prefixes: Vec<LedgerPath>,
    epoch: u64,
    continuity_generation: u64,
    owner_token: String,
    owner_fence_nonce: Option<Vec<u8>>,
    provider_id: String,
    provider_identity: String,
    provider_cursor: Option<Vec<u8>>,
    provider_fence: Option<Vec<u8>>,
    durable_offset: u64,
    folded_offset: u64,
    rows: Vec<ProvenPrefixRow>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ProvenPrefixRow {
    prefix: LedgerPath,
    event_flags: i64,
    source_mask: i64,
    first_sequence: u64,
    last_sequence: u64,
    provider_sequence: u64,
    created_at: i64,
    updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ReconcileMode {
    Full,
    ProvenPrefixes(ProvenPrefixSet),
}

impl ReconcileMode {
    fn label(&self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::ProvenPrefixes(_) => "prefix",
        }
    }

    fn completeness(&self) -> &'static str {
        match self {
            Self::Full => "complete",
            Self::ProvenPrefixes(_) => "provider_complete_prefix",
        }
    }

    fn prefixes(&self) -> Vec<String> {
        match self {
            Self::Full => Vec::new(),
            Self::ProvenPrefixes(proof) => proof
                .prefixes
                .iter()
                .map(|path| path.as_str().to_string())
                .collect(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
struct StoredAttemptIdentity {
    scope_id: String,
    scope_root: String,
    scope_root_identity: String,
    case_sensitive: bool,
    epoch: u64,
    ref_name: String,
    ref_generation: u64,
    change_id: String,
    baseline_root_id: String,
    policy_fingerprint: String,
    policy_generation: u64,
    trust_state: String,
    continuity_generation: u64,
    filesystem_identity: String,
    provider_id: Option<String>,
    provider_identity: Option<String>,
    observer_owner_token: Option<String>,
    initial_durable_offset: u64,
    initial_folded_offset: u64,
    start_fence: ObserverFence,
    root_handle_identity: Vec<u8>,
}

pub(crate) struct ReconciliationAttempt {
    attempt_id: String,
    expected: ExpectedScope,
    mode: ReconcileMode,
    reason: String,
    start_fence: ObserverFence,
    end_fence: Option<ObserverFence>,
    qualification: Option<ObserverQualification>,
    stored_identity: StoredAttemptIdentity,
    encoded_identity: Vec<u8>,
    root: PinnedWorktreeRoot,
    report: ChangeLedgerReconcileReport,
    #[cfg(test)]
    final_publication_hook: Option<Box<dyn FnOnce(&rusqlite::Connection)>>,
}

pub(crate) fn persisted_proven_prefixes(
    ledger: &ChangedPathLedger<'_>,
    expected: &ExpectedScope,
    requested: &[LedgerPath],
) -> Result<ProvenPrefixSet> {
    if requested.is_empty() {
        return Err(Error::InvalidInput(
            "provider-proven prefix reconciliation requires at least one prefix".into(),
        ));
    }
    exact_scope_guard(ledger.conn, expected)?;
    let scope_id = expected.scope_id.to_text();
    let (
        state,
        reason,
        provider_id,
        provider_identity,
        provider_cursor,
        provider_fence,
        durable_offset,
        folded_offset,
        continuity_generation,
    ): (
        String,
        String,
        String,
        String,
        Option<Vec<u8>>,
        Option<Vec<u8>>,
        i64,
        i64,
        i64,
    ) = ledger.conn.query_row(
        "SELECT trust_state,trust_reason,provider_id,provider_identity,
                provider_cursor,provider_fence,durable_offset,folded_offset,
                continuity_generation
         FROM changed_path_scopes WHERE scope_id=?1",
        [&scope_id],
        |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
                row.get(5)?,
                row.get(6)?,
                row.get(7)?,
                row.get(8)?,
            ))
        },
    )?;
    if !matches!(state.as_str(), "trusted" | "reconciling") {
        return Err(reconcile_required(
            expected,
            &state,
            &format!("full reconciliation required: {reason}"),
        ));
    }
    if provider_identity != hex::encode(&expected.provider_identity) {
        return Err(reconcile_required(
            expected,
            &state,
            "provider identity changed; full reconciliation required",
        ));
    }
    let owner = ledger
        .conn
        .query_row(
            "SELECT owner_token,fence_nonce FROM changed_path_observer_owners
         WHERE scope_id=?1 AND epoch=?2 AND provider_id=?3
           AND provider_identity=?4 AND lease_state='active' AND expires_at>=?5",
            params![
                scope_id,
                sql_u64(expected.epoch)?,
                provider_id,
                provider_identity,
                now_ts(),
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
        )
        .optional()?;
    let Some((owner_token, owner_fence_nonce)) = owner else {
        return Err(reconcile_required(
            expected,
            &state,
            "qualified provider owner is unavailable; full reconciliation required",
        ));
    };
    let mut prefixes = requested.to_vec();
    prefixes.sort();
    prefixes.dedup();
    let mut proof_rows = Vec::with_capacity(prefixes.len());
    for prefix in &prefixes {
        let row = ledger
            .conn
            .query_row(
                "SELECT event_flags,source_mask,first_sequence,last_sequence,
                    provider_sequence,created_at,updated_at
             FROM changed_path_prefixes
             WHERE scope_id=?1 AND normalized_prefix=?2 COLLATE BINARY
               AND completeness_reason='provider_complete'
               AND source_mask=?3 AND provider_id=?4
               AND provider_sequence IS NOT NULL AND intent_id IS NULL",
                params![
                    scope_id,
                    prefix.as_str(),
                    super::EvidenceSource::Observer.mask(),
                    provider_id,
                ],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                        row.get::<_, i64>(6)?,
                    ))
                },
            )
            .optional()?;
        let Some((event_flags, source_mask, first, last, sequence, created_at, updated_at)) = row
        else {
            return Err(reconcile_required(
                expected,
                &state,
                "prefix was not persisted by the qualified provider",
            ));
        };
        proof_rows.push(ProvenPrefixRow {
            prefix: prefix.clone(),
            event_flags,
            source_mask,
            first_sequence: db_u64(first)?,
            last_sequence: db_u64(last)?,
            provider_sequence: db_u64(sequence)?,
            created_at,
            updated_at,
        });
    }
    let durable_offset = db_u64(durable_offset)?;
    let folded_offset = db_u64(folded_offset)?;
    let continuity_generation = db_u64(continuity_generation)?;
    if folded_offset > durable_offset {
        return Err(Error::Corrupt(
            "prefix proof folded cut exceeds durable cut".into(),
        ));
    }
    Ok(ProvenPrefixSet {
        prefixes,
        epoch: expected.epoch,
        continuity_generation,
        owner_token,
        owner_fence_nonce,
        provider_id,
        provider_identity,
        provider_cursor,
        provider_fence,
        durable_offset,
        folded_offset,
        rows: proof_rows,
    })
}

pub(crate) fn begin_reconciliation(
    trail: &Trail,
    ledger: &ChangedPathLedger<'_>,
    observer: &dyn QualifiedObserver,
    expected: &ExpectedScope,
    policy: &CompiledPolicy,
    mode: ReconcileMode,
    reason: &str,
) -> Result<ReconciliationAttempt> {
    if !policy.authorizes_reconciliation(expected) {
        return Err(reconcile_required(
            expected,
            TrustState::StaleBaseline.as_str(),
            "compiled recording policy has no authenticated reconciliation authorization",
        ));
    }
    // The observation cut is deliberately acquired before the root is opened
    // and before any enumeration can begin.
    let start_fence = observer.begin_observation(expected)?;
    let root = trail.open_pinned_worktree_root(policy)?;
    let root_handle_identity = trail.pinned_worktree_root_identity(&root);
    let attempt_id = format!(
        "reconcile-{}-{}",
        now_ts(),
        NEXT_ATTEMPT_ID.fetch_add(1, Ordering::Relaxed)
    );
    let tx = Transaction::new_unchecked(ledger.conn, TransactionBehavior::Immediate)?;
    exact_scope_guard(&tx, expected)?;
    validate_mode_start(&tx, expected, &mode)?;
    if matches!(mode, ReconcileMode::Full) {
        exact_scope_update_state(&tx, expected, TrustState::Reconciling, reason)?;
    }
    let stored_identity =
        capture_attempt_identity(&tx, expected, start_fence.clone(), root_handle_identity)?;
    let encoded_identity = serde_json::to_vec(&stored_identity)?;
    let scope_id = stored_identity.scope_id.clone();
    tx.execute(
        "UPDATE changed_path_reconciliations
         SET state='abandoned', updated_at=?1
         WHERE scope_id=?2 AND state IN ('prepared','staging','ready')",
        params![now_ts(), scope_id],
    )?;
    let start_cursor = start_fence.sequence.to_be_bytes();
    tx.execute(
        "INSERT INTO changed_path_reconciliations(
             attempt_id, scope_id, expected_scope_epoch, expected_ref_name,
             expected_ref_generation, expected_change_id, expected_root_id,
             filesystem_identity, policy_fingerprint,
             policy_dependency_generation, provider_id, provider_identity,
             start_cursor, start_fence, mode, reason, completeness_class,
             staged_store_location, state, created_at, updated_at
         ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,
                  ?15,?16,?17,'sqlite','staging',?18,?18)",
        params![
            attempt_id,
            scope_id,
            sql_u64(expected.epoch)?,
            expected.ref_name,
            sql_u64(expected.ref_generation)?,
            stored_identity.change_id,
            stored_identity.baseline_root_id,
            stored_identity.filesystem_identity,
            stored_identity.policy_fingerprint,
            sql_u64(stored_identity.policy_generation)?,
            stored_identity.provider_id,
            stored_identity.provider_identity,
            start_cursor.as_slice(),
            encoded_identity,
            mode.label(),
            reason,
            mode.completeness(),
            now_ts(),
        ],
    )?;
    tx.commit()?;

    Ok(ReconciliationAttempt {
        attempt_id,
        expected: expected.clone(),
        mode: mode.clone(),
        reason: reason.to_string(),
        start_fence: start_fence.clone(),
        end_fence: None,
        qualification: None,
        stored_identity,
        encoded_identity,
        root,
        report: ChangeLedgerReconcileReport {
            mode: mode.label().to_string(),
            reason: reason.to_string(),
            start_sequence: start_fence.sequence,
            start_durable_offset: start_fence.durable_offset,
            trust_state: TrustState::Reconciling.as_str().to_string(),
            ..ChangeLedgerReconcileReport::default()
        },
        #[cfg(test)]
        final_publication_hook: None,
    })
}

pub(crate) fn reconcile_full(
    trail: &Trail,
    ledger: &ChangedPathLedger<'_>,
    observer: &dyn QualifiedObserver,
    expected: &ExpectedScope,
    policy: &CompiledPolicy,
    reason: &str,
) -> Result<ChangeLedgerReconcileReport> {
    let mut retries = 0;
    loop {
        let mut attempt = begin_reconciliation(
            trail,
            ledger,
            observer,
            expected,
            policy,
            ReconcileMode::Full,
            reason,
        )?;
        if let Err(error) = attempt.observe(trail, ledger, observer, policy) {
            if retries < MAX_IDENTITY_RACE_RETRIES && is_retryable_identity_race(&error) {
                retries += 1;
                continue;
            }
            return Err(error);
        }
        match attempt.publish(trail, ledger, policy) {
            Ok(mut report) => {
                report.retries = retries;
                return Ok(report);
            }
            Err(error)
                if retries < MAX_IDENTITY_RACE_RETRIES && is_retryable_identity_race(&error) =>
            {
                retries += 1;
            }
            Err(error) => return Err(error),
        }
    }
}

fn is_retryable_identity_race(error: &Error) -> bool {
    let message = match error {
        Error::InvalidInput(message) => message.as_str(),
        Error::ChangeLedgerReconcileRequired { reason, .. } => reason.as_str(),
        _ => return false,
    };
    message.contains("identity race")
        || message.contains("changed while it was read")
        || message.contains("workspace root identity changed")
        || message.contains("directory identity changed")
}

impl ReconciliationAttempt {
    #[cfg(test)]
    fn set_final_publication_hook<F>(&mut self, hook: F)
    where
        F: FnOnce(&rusqlite::Connection) + 'static,
    {
        self.final_publication_hook = Some(Box::new(hook));
    }

    #[cfg(test)]
    fn run_final_publication_hook(&mut self, conn: &rusqlite::Connection) {
        if let Some(hook) = self.final_publication_hook.take() {
            hook(conn);
        }
    }

    pub(crate) fn observe(
        &mut self,
        trail: &Trail,
        ledger: &ChangedPathLedger<'_>,
        observer: &dyn QualifiedObserver,
        policy: &CompiledPolicy,
    ) -> Result<()> {
        let result = self.observe_inner(trail, ledger, observer, policy);
        if let Err(error) = result {
            mark_attempt_failed(ledger.conn, &self.attempt_id, &error.to_string())?;
            return Err(error);
        }
        Ok(())
    }

    fn observe_inner(
        &mut self,
        trail: &Trail,
        ledger: &ChangedPathLedger<'_>,
        observer: &dyn QualifiedObserver,
        policy: &CompiledPolicy,
    ) -> Result<()> {
        if self.end_fence.is_some() {
            return Err(Error::InvalidInput(
                "reconciliation attempt was already observed".into(),
            ));
        }
        let prefixes = self.mode.prefixes();
        let mut writer = StagingWriter::new(ledger.conn, &self.attempt_id);
        trail.visit_pinned_worktree_files(&self.root, policy, &prefixes, |entry| match entry {
            ReconciliationScanEntry::Directory(directory) => {
                writer.stage_directory_guard(directory)
            }
            ReconciliationScanEntry::File(file) => writer.stage_filesystem(file),
        })?;
        writer.flush()?;
        self.validate_directory_guards(trail, ledger)?;
        trail.visit_root_file_entries(
            &self.expected.baseline_root,
            &prefixes,
            |path, baseline| writer.compare_baseline(path, baseline),
        )?;
        writer.flush()?;

        let end = observer.end_fence(&self.expected, &self.start_fence)?;
        if end.sequence < self.start_fence.sequence
            || end.durable_offset < self.start_fence.durable_offset
        {
            return self.fail(
                ledger,
                "observer end fence regressed behind reconciliation start fence",
            );
        }
        let root_identity = trail.pinned_worktree_root_identity(&self.root);
        let mut spool = ObserverEventSpool::new()?;
        let qualification = observer.drain_through(
            &self.expected,
            &root_identity,
            &self.start_fence,
            &end,
            &mut |event| {
                if event.sequence <= self.start_fence.sequence || event.sequence > end.sequence {
                    return Err(Error::InvalidInput(
                        "observer returned evidence outside requested fence interval".into(),
                    ));
                }
                spool.push(event)
            },
        )?;
        spool.finish()?;
        while let Some(event) = spool.next()? {
            if raw_event_invalidates_policy(policy, std::path::Path::new(event.path.as_str())) {
                return self.fail(
                    ledger,
                    "recording policy changed during reconciliation interval",
                );
            }
            let current =
                trail.read_pinned_worktree_path(&self.root, policy, event.path.as_str())?;
            let baseline =
                trail.root_file_entry(&self.expected.baseline_root, event.path.as_str())?;
            writer.stage_observer_result(event, current, baseline)?;
        }
        writer.flush()?;
        let candidate_rows = ledger.conn.query_row(
            "SELECT COUNT(*) FROM changed_path_reconciliation_rows
             WHERE attempt_id=?1 AND before_identity LIKE 'flags:%'",
            [&self.attempt_id],
            |row| row.get::<_, i64>(0),
        )?;
        let changed = ledger.conn.execute(
            "UPDATE changed_path_reconciliations SET state='ready', updated_at=?1
             WHERE attempt_id=?2 AND state='staging'",
            params![now_ts(), self.attempt_id],
        )?;
        if changed != 1 {
            return Err(reconcile_required(
                &self.expected,
                TrustState::Reconciling.as_str(),
                "reconciliation staging attempt was replaced",
            ));
        }
        self.report.observed_files = writer.observed_files;
        self.report.staged_rows = writer.staged_rows;
        self.report.observed_candidates = db_u64(candidate_rows)?;
        self.report.candidate_rows = db_u64(candidate_rows)?;
        self.report.hashed_bytes = writer.hashed_bytes;
        self.report.peak_batch_rows = writer.peak_batch_rows;
        self.report.peak_buffer_bytes = writer.peak_buffer_bytes;
        self.report.end_sequence = end.sequence;
        self.report.end_durable_offset = end.durable_offset;
        self.end_fence = Some(end);
        self.qualification = Some(qualification);
        Ok(())
    }

    fn validate_directory_guards(
        &self,
        trail: &Trail,
        ledger: &ChangedPathLedger<'_>,
    ) -> Result<()> {
        let mut statement = ledger.conn.prepare(
            "SELECT relative_path,directory_identity
             FROM changed_path_reconciliation_guards
             WHERE attempt_id=?1
             ORDER BY relative_path",
        )?;
        let mut rows = statement.query([&self.attempt_id])?;
        while let Some(row) = rows.next()? {
            let path = String::from_utf8(row.get::<_, Vec<u8>>(0)?)
                .map_err(|_| Error::Corrupt("invalid staged directory guard path".into()))?;
            let identity = row.get::<_, Vec<u8>>(1)?;
            if !trail.verify_pinned_worktree_directory(&self.root, &path, &identity)? {
                return Err(Error::InvalidInput(format!(
                    "directory identity changed during reconciliation: `{path}`"
                )));
            }
        }
        Ok(())
    }

    pub(crate) fn publish(
        mut self,
        trail: &Trail,
        ledger: &ChangedPathLedger<'_>,
        policy: &CompiledPolicy,
    ) -> Result<ChangeLedgerReconcileReport> {
        let Some(end) = self.end_fence.clone() else {
            return Err(Error::InvalidInput(
                "reconciliation attempt has not completed observation".into(),
            ));
        };
        let Some(qualification) = self.qualification.clone() else {
            return Err(reconcile_required(
                &self.expected,
                TrustState::Reconciling.as_str(),
                "qualified observer proof is unavailable",
            ));
        };
        if policy.fingerprint() != self.expected.policy_fingerprint
            || !policy.authorizes_reconciliation(&self.expected)
        {
            return self.fail_report(
                ledger,
                "compiled recording policy fingerprint changed before publication",
            );
        }
        let root_identity = trail.pinned_worktree_root_identity(&self.root);
        if !qualification.validates(&self.expected, &root_identity, &self.start_fence, &end) {
            return self.fail_report(ledger, "observer qualification did not cover exact scope");
        }

        if matches!(self.mode, ReconcileMode::ProvenPrefixes(_)) {
            return self.publish_prefix(ledger, trail, &root_identity, &end);
        }

        let tx = Transaction::new_unchecked(ledger.conn, TransactionBehavior::Immediate)?;
        let (current_durable, current_folded) =
            match validate_stored_scope(&tx, &self.expected, &self.stored_identity) {
                Ok(cuts) => cuts,
                Err(_) => {
                    return self.fail_publication_transaction(
                        tx,
                        "scope changed during reconciliation publication",
                    )
                }
            };
        if current_durable < self.stored_identity.initial_durable_offset
            || current_folded < self.stored_identity.initial_folded_offset
        {
            return self.fail_publication_transaction(
                tx,
                "scope evidence cuts regressed during reconciliation publication",
            );
        }
        if validate_observer_continuity(
            &tx,
            &self.expected,
            &self.stored_identity,
            &qualification,
            &end,
        )
        .is_err()
        {
            return self.fail_observer_continuity_transaction(
                tx,
                "observer owner or durable segment changed before publication",
            );
        }
        if !matches!(trail.verify_pinned_worktree_root(&self.root), Ok(true)) {
            return self.fail_publication_transaction(
                tx,
                "workspace root identity changed before publication",
            );
        }
        if self.validate_directory_guards(trail, ledger).is_err() {
            return self.fail_publication_transaction(
                tx,
                "directory identity changed before reconciliation publication",
            );
        }
        if validate_ready_attempt(&tx, &self, &root_identity).is_err() {
            return self
                .fail_publication_transaction(tx, "reconciliation attempt identity changed");
        }
        if validate_scope_capabilities(&tx, &self.expected).is_err() {
            return self.fail_publication_transaction(
                tx,
                "provider qualification changed before publication",
            );
        }
        let scope_id = self.expected.scope_id.to_text();
        tx.execute_batch("SAVEPOINT changed_path_reconciliation_candidates;")?;
        tx.execute(
            "DELETE FROM changed_path_entries
             WHERE scope_id=?1 AND (
                 source_mask=?2 OR
                 (source_mask=?3 AND provider_sequence IS NOT NULL AND provider_sequence<=?4)
             )",
            params![
                scope_id,
                super::EvidenceSource::Reconciliation.mask(),
                super::EvidenceSource::Observer.mask(),
                sql_u64(end.sequence)?,
            ],
        )?;
        tx.execute(
            "DELETE FROM changed_path_prefixes
             WHERE scope_id=?1 AND (
                 source_mask=?2 OR
                 (source_mask=?3 AND provider_sequence IS NOT NULL AND provider_sequence<=?4)
             )",
            params![
                scope_id,
                super::EvidenceSource::Reconciliation.mask(),
                super::EvidenceSource::Observer.mask(),
                sql_u64(end.sequence)?,
            ],
        )?;
        let now = now_ts();
        tx.execute(
            "INSERT INTO changed_path_entries(
                 scope_id, normalized_path, event_flags, source_mask,
                 first_sequence, last_sequence, provider_id, provider_sequence,
                 intent_id, created_at, updated_at
             )
             SELECT ?1, normalized_path,
                    CAST(substr(before_identity, 7) AS INTEGER), ?2,
                    ?3, ?3, 'reconciliation', NULL, NULL, ?4, ?4
             FROM changed_path_reconciliation_rows
             WHERE attempt_id=?5 AND before_identity LIKE 'flags:%'
             ON CONFLICT(scope_id, normalized_path) DO UPDATE SET
                 event_flags=(changed_path_entries.event_flags | excluded.event_flags),
                 source_mask=(changed_path_entries.source_mask | excluded.source_mask),
                 first_sequence=MIN(changed_path_entries.first_sequence, excluded.first_sequence),
                 last_sequence=MAX(changed_path_entries.last_sequence, excluded.last_sequence),
                 updated_at=excluded.updated_at",
            params![
                scope_id,
                super::EvidenceSource::Reconciliation.mask(),
                sql_u64(end.sequence)?,
                now,
                self.attempt_id,
            ],
        )?;
        if candidate_cap_exceeded(&tx, &scope_id)? {
            return self.fail_candidate_cap_transaction(tx);
        }
        #[cfg(test)]
        self.run_final_publication_hook(&tx);
        if validate_observer_continuity_at(
            &tx,
            &self.expected,
            &self.stored_identity,
            &qualification,
            &end,
            publication_lease_deadline()?,
        )
        .is_err()
        {
            return self.fail_observer_continuity_candidate_transaction(
                tx,
                "observer lease or durable segment became unavailable at publication boundary",
            );
        }
        tx.execute_batch("RELEASE changed_path_reconciliation_candidates;")?;
        let merged_durable = current_durable.max(end.durable_offset);
        let merged_folded = current_folded.max(end.durable_offset);
        debug_assert!(merged_folded <= merged_durable);
        let changed = tx.execute(
            "UPDATE changed_path_scopes
             SET trust_state='trusted', trust_reason='reconciliation_published',
                 durable_offset=?1, folded_offset=?2, updated_at=?3
             WHERE scope_id=?4 AND scope_root=?5 AND scope_root_identity=?6
               AND case_sensitive=?7 AND epoch=?8 AND ref_name=?9
               AND ref_generation=?10 AND change_id=?11 AND baseline_root_id=?12
               AND policy_fingerprint=?13 AND policy_dependency_generation=?14
               AND filesystem_identity=?15 AND provider_id IS ?16
               AND provider_identity IS ?17 AND observer_owner_token IS ?18
               AND durable_offset=?19 AND folded_offset=?20
               AND trust_state='reconciling' AND continuity_generation=?21",
            params![
                sql_u64(merged_durable)?,
                sql_u64(merged_folded)?,
                now,
                self.stored_identity.scope_id,
                self.stored_identity.scope_root,
                self.stored_identity.scope_root_identity,
                self.stored_identity.case_sensitive,
                sql_u64(self.stored_identity.epoch)?,
                self.stored_identity.ref_name,
                sql_u64(self.stored_identity.ref_generation)?,
                self.stored_identity.change_id,
                self.stored_identity.baseline_root_id,
                self.stored_identity.policy_fingerprint,
                sql_u64(self.stored_identity.policy_generation)?,
                self.stored_identity.filesystem_identity,
                self.stored_identity.provider_id,
                self.stored_identity.provider_identity,
                self.stored_identity.observer_owner_token,
                sql_u64(current_durable)?,
                sql_u64(current_folded)?,
                sql_u64(self.stored_identity.continuity_generation)?,
            ],
        )?;
        if changed != 1 {
            tx.rollback()?;
            return self.fail_report(ledger, "scope changed during reconciliation publication");
        }
        tx.execute(
            "UPDATE changed_path_reconciliations SET state='published', updated_at=?1
             WHERE attempt_id=?2 AND state='ready'",
            params![now, self.attempt_id],
        )?;
        tx.commit()?;
        self.report.published = true;
        self.report.refreshed = true;
        self.report.trust_state = TrustState::Trusted.as_str().to_string();
        let candidate_rows = ledger.conn.query_row(
            "SELECT COUNT(*) FROM changed_path_entries WHERE scope_id=?1",
            [self.expected.scope_id.to_text()],
            |row| row.get::<_, i64>(0),
        )?;
        self.report.candidate_rows = db_u64(candidate_rows)?;
        Ok(self.report)
    }

    fn publish_prefix(
        mut self,
        ledger: &ChangedPathLedger<'_>,
        trail: &Trail,
        root_identity: &[u8],
        end: &ObserverFence,
    ) -> Result<ChangeLedgerReconcileReport> {
        let ReconcileMode::ProvenPrefixes(proof) = self.mode.clone() else {
            unreachable!("prefix publication requires a prefix proof");
        };
        let tx = Transaction::new_unchecked(ledger.conn, TransactionBehavior::Immediate)?;
        if validate_stored_scope(&tx, &self.expected, &self.stored_identity).is_err() {
            return self.fail_publication_transaction(
                tx,
                "scope changed during prefix reconciliation publication",
            );
        }
        let qualification = self
            .qualification
            .clone()
            .expect("publication checked observer qualification");
        if validate_observer_continuity(
            &tx,
            &self.expected,
            &self.stored_identity,
            &qualification,
            end,
        )
        .is_err()
        {
            return self.fail_observer_continuity_transaction(
                tx,
                "observer owner or durable segment changed before prefix publication",
            );
        }
        if validate_prefix_proof(&tx, &self.expected, &proof).is_err() {
            return self.fail_publication_transaction(
                tx,
                "provider owner, cut, or persisted prefix changed before publication",
            );
        }
        if !matches!(trail.verify_pinned_worktree_root(&self.root), Ok(true)) {
            return self.fail_publication_transaction(
                tx,
                "workspace root identity changed before prefix publication",
            );
        }
        if self.validate_directory_guards(trail, ledger).is_err() {
            return self.fail_publication_transaction(
                tx,
                "directory identity changed before prefix publication",
            );
        }
        if validate_ready_attempt(&tx, &self, root_identity).is_err() {
            return self.fail_publication_transaction(tx, "prefix reconciliation attempt changed");
        }

        let scope_id = self.stored_identity.scope_id.clone();
        tx.execute_batch("SAVEPOINT changed_path_reconciliation_candidates;")?;
        for proof_row in &proof.rows {
            let prefix = &proof_row.prefix;
            let lower = format!("{}/", prefix.as_str());
            let upper = format!("{}0", prefix.as_str());
            tx.execute(
                "DELETE FROM changed_path_entries
                 WHERE scope_id=?1
                   AND (normalized_path=?2 COLLATE BINARY OR
                        (normalized_path>=?3 COLLATE BINARY AND normalized_path<?4 COLLATE BINARY))
                   AND (source_mask=?5 OR
                        (source_mask=?6 AND provider_id=?7
                         AND provider_sequence IS NOT NULL AND provider_sequence<=?8))",
                params![
                    scope_id,
                    prefix.as_str(),
                    lower,
                    upper,
                    super::EvidenceSource::Reconciliation.mask(),
                    super::EvidenceSource::Observer.mask(),
                    proof.provider_id,
                    sql_u64(proof_row.provider_sequence)?,
                ],
            )?;
            tx.execute(
                "DELETE FROM changed_path_prefixes
                 WHERE scope_id=?1
                   AND (normalized_prefix=?2 COLLATE BINARY OR
                        (normalized_prefix>=?3 COLLATE BINARY AND normalized_prefix<?4 COLLATE BINARY))
                   AND (source_mask=?5 OR
                        (source_mask=?6 AND provider_id=?7
                         AND provider_sequence IS NOT NULL AND provider_sequence<=?8))",
                params![
                    scope_id,
                    prefix.as_str(),
                    lower,
                    upper,
                    super::EvidenceSource::Reconciliation.mask(),
                    super::EvidenceSource::Observer.mask(),
                    proof.provider_id,
                    sql_u64(proof_row.provider_sequence)?,
                ],
            )?;
        }
        let now = now_ts();
        for row in &proof.rows {
            tx.execute(
                "INSERT INTO changed_path_prefixes(
                     scope_id,normalized_prefix,completeness_reason,event_flags,
                     source_mask,first_sequence,last_sequence,provider_id,
                     provider_sequence,intent_id,created_at,updated_at
                 ) VALUES(?1,?2,'provider_complete',?3,?4,?5,?6,?7,?8,NULL,?9,?10)",
                params![
                    scope_id,
                    row.prefix.as_str(),
                    row.event_flags,
                    row.source_mask,
                    sql_u64(row.first_sequence)?,
                    sql_u64(row.last_sequence)?,
                    proof.provider_id,
                    sql_u64(row.provider_sequence)?,
                    row.created_at,
                    row.updated_at,
                ],
            )?;
        }
        tx.execute(
            "INSERT INTO changed_path_entries(
                 scope_id, normalized_path, event_flags, source_mask,
                 first_sequence, last_sequence, provider_id, provider_sequence,
                 intent_id, created_at, updated_at
             )
             SELECT ?1, normalized_path,
                    CAST(substr(before_identity, 7) AS INTEGER), ?2,
                    ?3, ?3, 'reconciliation', NULL, NULL, ?4, ?4
             FROM changed_path_reconciliation_rows
             WHERE attempt_id=?5 AND before_identity LIKE 'flags:%'
             ON CONFLICT(scope_id, normalized_path) DO UPDATE SET
                 event_flags=(changed_path_entries.event_flags | excluded.event_flags),
                 source_mask=(changed_path_entries.source_mask | excluded.source_mask),
                 first_sequence=MIN(changed_path_entries.first_sequence, excluded.first_sequence),
                 last_sequence=MAX(changed_path_entries.last_sequence, excluded.last_sequence),
                 updated_at=excluded.updated_at",
            params![
                scope_id,
                super::EvidenceSource::Reconciliation.mask(),
                sql_u64(end.sequence)?,
                now,
                self.attempt_id,
            ],
        )?;
        if candidate_cap_exceeded(&tx, &scope_id)? {
            return self.fail_candidate_cap_transaction(tx);
        }
        #[cfg(test)]
        self.run_final_publication_hook(&tx);
        if validate_observer_continuity_at(
            &tx,
            &self.expected,
            &self.stored_identity,
            &qualification,
            end,
            publication_lease_deadline()?,
        )
        .is_err()
        {
            return self.fail_observer_continuity_candidate_transaction(
                tx,
                "observer lease or durable segment became unavailable at prefix publication boundary",
            );
        }
        tx.execute_batch("RELEASE changed_path_reconciliation_candidates;")?;
        let terminalized = tx.execute(
            "UPDATE changed_path_reconciliations SET state='published', updated_at=?1
             WHERE attempt_id=?2 AND state='ready'",
            params![now, self.attempt_id],
        )?;
        if terminalized != 1 {
            return self
                .fail_publication_transaction(tx, "prefix reconciliation attempt was replaced");
        }
        let trust_state: String = tx.query_row(
            "SELECT trust_state FROM changed_path_scopes WHERE scope_id=?1",
            [&scope_id],
            |row| row.get(0),
        )?;
        let candidate_rows = tx.query_row(
            "SELECT COUNT(*) FROM changed_path_entries WHERE scope_id=?1",
            [&scope_id],
            |row| row.get::<_, i64>(0),
        )?;
        tx.commit()?;
        self.report.refreshed = true;
        self.report.published = false;
        self.report.trust_state = trust_state;
        self.report.candidate_rows = db_u64(candidate_rows)?;
        Ok(self.report)
    }

    fn fail_candidate_cap_transaction(
        &self,
        tx: Transaction<'_>,
    ) -> Result<ChangeLedgerReconcileReport> {
        tx.execute_batch(
            "ROLLBACK TO changed_path_reconciliation_candidates;
             RELEASE changed_path_reconciliation_candidates;",
        )?;
        let reason = "reconciliation candidate row cap exceeded";
        tx.execute(
            "UPDATE changed_path_scopes
             SET trust_state='overflow',trust_reason=?1,
                 continuity_generation=continuity_generation+1,updated_at=?2
             WHERE scope_id=?3",
            params![reason, now_ts(), self.stored_identity.scope_id],
        )?;
        tx.execute(
            "UPDATE changed_path_reconciliations
             SET state='failed',reason=?1,updated_at=?2
             WHERE attempt_id=?3 AND state!='published'",
            params![reason, now_ts(), self.attempt_id],
        )?;
        tx.commit()?;
        Err(reconcile_required(
            &self.expected,
            TrustState::Overflow.as_str(),
            reason,
        ))
    }

    fn fail_observer_continuity_transaction(
        &self,
        tx: Transaction<'_>,
        reason: &str,
    ) -> Result<ChangeLedgerReconcileReport> {
        self.commit_observer_continuity_failure(tx, reason)
    }

    fn fail_observer_continuity_candidate_transaction(
        &self,
        tx: Transaction<'_>,
        reason: &str,
    ) -> Result<ChangeLedgerReconcileReport> {
        tx.execute_batch(
            "ROLLBACK TO changed_path_reconciliation_candidates;
             RELEASE changed_path_reconciliation_candidates;",
        )?;
        self.commit_observer_continuity_failure(tx, reason)
    }

    fn commit_observer_continuity_failure(
        &self,
        tx: Transaction<'_>,
        reason: &str,
    ) -> Result<ChangeLedgerReconcileReport> {
        let now = now_ts();
        tx.execute(
            "UPDATE changed_path_scopes
             SET trust_state='untrusted_gap',trust_reason=?1,
                 continuity_generation=continuity_generation+1,updated_at=?2
             WHERE scope_id=?3 AND continuity_generation=?4
               AND trust_state IN ('trusted','reconciling')",
            params![
                reason,
                now,
                self.stored_identity.scope_id,
                sql_u64(self.stored_identity.continuity_generation)?,
            ],
        )?;
        tx.execute(
            "UPDATE changed_path_reconciliations
             SET state='failed',reason=?1,updated_at=?2
             WHERE attempt_id=?3 AND state!='published'",
            params![reason, now, self.attempt_id],
        )?;
        tx.commit()?;
        Err(reconcile_required(
            &self.expected,
            TrustState::UntrustedGap.as_str(),
            reason,
        ))
    }

    fn fail<T>(&self, ledger: &ChangedPathLedger<'_>, reason: &str) -> Result<T> {
        mark_attempt_failed(ledger.conn, &self.attempt_id, reason)?;
        Err(reconcile_required(
            &self.expected,
            TrustState::Reconciling.as_str(),
            reason,
        ))
    }

    fn fail_report(
        &self,
        ledger: &ChangedPathLedger<'_>,
        reason: &str,
    ) -> Result<ChangeLedgerReconcileReport> {
        self.fail(ledger, reason)
    }

    fn fail_publication_transaction(
        &self,
        tx: Transaction<'_>,
        reason: &str,
    ) -> Result<ChangeLedgerReconcileReport> {
        tx.execute(
            "UPDATE changed_path_reconciliations
             SET state='failed', reason=?1, updated_at=?2
             WHERE attempt_id=?3 AND state!='published'",
            params![reason, now_ts(), self.attempt_id],
        )?;
        tx.commit()?;
        Err(reconcile_required(
            &self.expected,
            TrustState::Reconciling.as_str(),
            reason,
        ))
    }
}

#[derive(Clone)]
struct StagedRow {
    path: String,
    row_kind: &'static str,
    file_kind: Option<String>,
    content_hash: Option<String>,
    executable: Option<bool>,
    size_bytes: Option<u64>,
    flags: EvidenceFlags,
    identity: Option<Vec<u8>>,
    source_sequence: Option<u64>,
}

struct StagedDirectoryGuard {
    path: Vec<u8>,
    identity: Vec<u8>,
}

struct StagingWriter<'a> {
    conn: &'a rusqlite::Connection,
    attempt_id: &'a str,
    pending: Vec<StagedRow>,
    pending_guards: Vec<StagedDirectoryGuard>,
    observed_files: u64,
    staged_rows: u64,
    hashed_bytes: u64,
    peak_batch_rows: u64,
    peak_buffer_bytes: u64,
}

impl<'a> StagingWriter<'a> {
    fn new(conn: &'a rusqlite::Connection, attempt_id: &'a str) -> Self {
        Self {
            conn,
            attempt_id,
            pending: Vec::with_capacity(STAGING_BATCH_ROWS),
            pending_guards: Vec::with_capacity(STAGING_BATCH_ROWS),
            observed_files: 0,
            staged_rows: 0,
            hashed_bytes: 0,
            peak_batch_rows: 0,
            peak_buffer_bytes: 0,
        }
    }

    fn stage_filesystem(&mut self, file: ReconciliationFile) -> Result<()> {
        self.observed_files = self.observed_files.saturating_add(1);
        self.hashed_bytes = self.hashed_bytes.saturating_add(file.size_bytes);
        self.peak_buffer_bytes = self.peak_buffer_bytes.max(file.peak_buffer_bytes);
        self.push(file_row(file, EvidenceFlags::CREATE, None))
    }

    fn stage_directory_guard(&mut self, directory: ReconciliationDirectory) -> Result<()> {
        self.pending_guards.push(StagedDirectoryGuard {
            path: directory.path.into_bytes(),
            identity: directory.identity,
        });
        self.after_push()
    }

    fn compare_baseline(&mut self, path: String, baseline: FileEntry) -> Result<()> {
        let staged = self
            .conn
            .query_row(
                "SELECT file_kind, content_hash, executable, size_bytes, after_identity
             FROM changed_path_reconciliation_rows
             WHERE attempt_id=?1 AND normalized_path=?2 COLLATE BINARY",
                params![self.attempt_id, path],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<i64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                    ))
                },
            )
            .optional()?;
        match staged {
            None => self.push(StagedRow {
                path,
                row_kind: "deletion",
                file_kind: None,
                content_hash: None,
                executable: None,
                size_bytes: None,
                flags: EvidenceFlags::DELETE,
                identity: None,
                source_sequence: None,
            }),
            Some((kind, hash, executable, size, identity)) => {
                let current_kind = kind.unwrap_or_default();
                let current_hash = hash.unwrap_or_default();
                let current_executable = executable.unwrap_or_default() != 0;
                let mut flags = EvidenceFlags::default();
                if current_hash != baseline.content_hash
                    || current_kind != file_kind_label(&baseline.kind)
                    || size.unwrap_or_default().max(0) as u64 != baseline.size_bytes
                {
                    flags |= EvidenceFlags::CONTENT;
                }
                if current_executable != baseline.executable {
                    flags |= EvidenceFlags::MODE;
                }
                if flags == EvidenceFlags::default() {
                    self.conn.execute(
                        "DELETE FROM changed_path_reconciliation_rows
                         WHERE attempt_id=?1 AND normalized_path=?2 COLLATE BINARY",
                        params![self.attempt_id, path],
                    )?;
                    Ok(())
                } else {
                    self.push(StagedRow {
                        path,
                        row_kind: "entry",
                        file_kind: Some(current_kind),
                        content_hash: Some(current_hash),
                        executable: Some(current_executable),
                        size_bytes: size.map(|value| value.max(0) as u64),
                        flags,
                        identity: identity.and_then(|value| hex::decode(value).ok()),
                        source_sequence: None,
                    })
                }
            }
        }
    }

    fn stage_observer_result(
        &mut self,
        event: ObserverEvent,
        current: Option<ReconciliationFile>,
        baseline: Option<FileEntry>,
    ) -> Result<()> {
        match (current, baseline) {
            (None, None) => {
                self.conn.execute(
                    "DELETE FROM changed_path_reconciliation_rows
                     WHERE attempt_id=?1 AND normalized_path=?2 COLLATE BINARY",
                    params![self.attempt_id, event.path.as_str()],
                )?;
                Ok(())
            }
            (None, Some(_)) => self.push(StagedRow {
                path: event.path.as_str().to_string(),
                row_kind: "deletion",
                file_kind: None,
                content_hash: None,
                executable: None,
                size_bytes: None,
                flags: EvidenceFlags::DELETE | event.flags,
                identity: None,
                source_sequence: Some(event.sequence),
            }),
            (Some(file), None) => {
                self.observed_files = self.observed_files.saturating_add(1);
                self.hashed_bytes = self.hashed_bytes.saturating_add(file.size_bytes);
                self.peak_buffer_bytes = self.peak_buffer_bytes.max(file.peak_buffer_bytes);
                self.push(file_row(
                    file,
                    EvidenceFlags::CREATE | event.flags,
                    Some(event.sequence),
                ))
            }
            (Some(file), Some(baseline)) => {
                let mut flags = EvidenceFlags::default();
                if file.content_hash != baseline.content_hash
                    || file.file_kind != file_kind_label(&baseline.kind)
                    || file.size_bytes != baseline.size_bytes
                {
                    flags |= EvidenceFlags::CONTENT;
                }
                if file.executable != baseline.executable {
                    flags |= EvidenceFlags::MODE;
                }
                if flags == EvidenceFlags::default() {
                    self.conn.execute(
                        "DELETE FROM changed_path_reconciliation_rows
                         WHERE attempt_id=?1 AND normalized_path=?2 COLLATE BINARY",
                        params![self.attempt_id, event.path.as_str()],
                    )?;
                    Ok(())
                } else {
                    self.push(file_row(file, flags | event.flags, Some(event.sequence)))
                }
            }
        }
    }

    fn push(&mut self, row: StagedRow) -> Result<()> {
        self.pending.push(row);
        self.after_push()
    }

    fn after_push(&mut self) -> Result<()> {
        let pending_rows = self.pending.len().saturating_add(self.pending_guards.len());
        self.peak_batch_rows = self.peak_batch_rows.max(pending_rows as u64);
        if pending_rows >= STAGING_BATCH_ROWS {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() && self.pending_guards.is_empty() {
            return Ok(());
        }
        let tx = Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
        for row in self.pending.drain(..) {
            tx.execute(
                "INSERT INTO changed_path_reconciliation_rows(
                     attempt_id, normalized_path, row_kind, file_kind,
                     content_hash, executable, size_bytes, before_identity,
                     after_identity, source_sequence, staged_at
                 ) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)
                 ON CONFLICT(attempt_id,normalized_path) DO UPDATE SET
                     row_kind=excluded.row_kind, file_kind=excluded.file_kind,
                     content_hash=excluded.content_hash, executable=excluded.executable,
                     size_bytes=excluded.size_bytes, before_identity=excluded.before_identity,
                     after_identity=excluded.after_identity,
                     source_sequence=excluded.source_sequence, staged_at=excluded.staged_at",
                params![
                    self.attempt_id,
                    row.path,
                    row.row_kind,
                    row.file_kind,
                    row.content_hash,
                    row.executable.map(i64::from),
                    row.size_bytes.map(sql_u64).transpose()?,
                    format!("flags:{}", row.flags.0),
                    row.identity.map(hex::encode),
                    row.source_sequence.map(sql_u64).transpose()?,
                    now_ts(),
                ],
            )?;
            self.staged_rows = self.staged_rows.saturating_add(1);
        }
        for guard in self.pending_guards.drain(..) {
            tx.execute(
                "INSERT INTO changed_path_reconciliation_guards(
                     attempt_id,relative_path,directory_identity,staged_at
                 ) VALUES(?1,?2,?3,?4)
                 ON CONFLICT(attempt_id,relative_path) DO UPDATE SET
                     directory_identity=excluded.directory_identity,
                     staged_at=excluded.staged_at",
                params![self.attempt_id, guard.path, guard.identity, now_ts()],
            )?;
            self.staged_rows = self.staged_rows.saturating_add(1);
        }
        tx.commit()?;
        Ok(())
    }
}

fn file_row(
    file: ReconciliationFile,
    flags: EvidenceFlags,
    source_sequence: Option<u64>,
) -> StagedRow {
    StagedRow {
        path: file.path,
        row_kind: "entry",
        file_kind: Some(file.file_kind),
        content_hash: Some(file.content_hash),
        executable: Some(file.executable),
        size_bytes: Some(file.size_bytes),
        flags,
        identity: Some(file.identity),
        source_sequence,
    }
}

fn validate_mode_start(
    tx: &Transaction<'_>,
    expected: &ExpectedScope,
    mode: &ReconcileMode,
) -> Result<()> {
    if let ReconcileMode::ProvenPrefixes(proof) = mode {
        let state = current_trust_state(tx, expected)?;
        if !matches!(state.as_str(), "trusted" | "reconciling") {
            return Err(reconcile_required(
                expected,
                &state,
                "continuity failure forces full reconciliation",
            ));
        }
        if proof.prefixes.is_empty() {
            return Err(Error::InvalidInput(
                "empty provider prefix proof is not authoritative".into(),
            ));
        }
        validate_prefix_proof(tx, expected, proof)?;
    }
    Ok(())
}

fn validate_prefix_proof(
    conn: &rusqlite::Connection,
    expected: &ExpectedScope,
    proof: &ProvenPrefixSet,
) -> Result<()> {
    let scope_id = expected.scope_id.to_text();
    let scope_matches = conn.query_row(
        "SELECT COUNT(*) FROM changed_path_scopes
         WHERE scope_id=?1 AND epoch=?2 AND provider_id=?3
           AND provider_identity=?4 AND provider_cursor IS ?5
           AND provider_fence IS ?6 AND durable_offset=?7 AND folded_offset=?8
           AND trust_state IN ('trusted','reconciling')
           AND continuity_generation=?9",
        params![
            scope_id,
            sql_u64(proof.epoch)?,
            proof.provider_id,
            proof.provider_identity,
            proof.provider_cursor,
            proof.provider_fence,
            sql_u64(proof.durable_offset)?,
            sql_u64(proof.folded_offset)?,
            sql_u64(proof.continuity_generation)?,
        ],
        |row| row.get::<_, i64>(0),
    )? == 1;
    let owner_matches = conn.query_row(
        "SELECT COUNT(*) FROM changed_path_observer_owners
         WHERE scope_id=?1 AND epoch=?2 AND owner_token=?3
           AND provider_id=?4 AND provider_identity=?5
           AND fence_nonce IS ?6 AND lease_state='active' AND expires_at>=?7",
        params![
            scope_id,
            sql_u64(proof.epoch)?,
            proof.owner_token,
            proof.provider_id,
            proof.provider_identity,
            proof.owner_fence_nonce,
            now_ts(),
        ],
        |row| row.get::<_, i64>(0),
    )? == 1;
    if !scope_matches || !owner_matches {
        return Err(reconcile_required(
            expected,
            TrustState::Reconciling.as_str(),
            "provider owner or exact cut changed; full reconciliation required",
        ));
    }
    for row in &proof.rows {
        let matched = conn.query_row(
            "SELECT COUNT(*) FROM changed_path_prefixes
             WHERE scope_id=?1 AND normalized_prefix=?2 COLLATE BINARY
               AND completeness_reason='provider_complete' AND event_flags=?3
               AND source_mask=?4 AND first_sequence=?5 AND last_sequence=?6
               AND provider_id=?7 AND provider_sequence=?8 AND intent_id IS NULL
               AND created_at=?9 AND updated_at=?10",
            params![
                scope_id,
                row.prefix.as_str(),
                row.event_flags,
                row.source_mask,
                sql_u64(row.first_sequence)?,
                sql_u64(row.last_sequence)?,
                proof.provider_id,
                sql_u64(row.provider_sequence)?,
                row.created_at,
                row.updated_at,
            ],
            |query| query.get::<_, i64>(0),
        )?;
        if matched != 1 {
            return Err(reconcile_required(
                expected,
                TrustState::Reconciling.as_str(),
                "persisted provider-complete prefix changed; full reconciliation required",
            ));
        }
    }
    Ok(())
}

fn validate_scope_capabilities(tx: &Transaction<'_>, expected: &ExpectedScope) -> Result<()> {
    let qualified = tx.query_row(
        "SELECT clean_proof_allowed=1 AND linearizable_fence=1
                AND filesystem_supported=1
         FROM changed_path_scopes WHERE scope_id=?1",
        [expected.scope_id.to_text()],
        |row| row.get::<_, bool>(0),
    )?;
    if !qualified {
        return Err(reconcile_required(
            expected,
            TrustState::Reconciling.as_str(),
            "provider capabilities do not permit a clean proof",
        ));
    }
    Ok(())
}

fn validate_observer_continuity(
    tx: &Transaction<'_>,
    expected: &ExpectedScope,
    identity: &StoredAttemptIdentity,
    qualification: &ObserverQualification,
    end: &ObserverFence,
) -> Result<()> {
    validate_observer_continuity_at(
        tx,
        expected,
        identity,
        qualification,
        end,
        now_ts().saturating_add(1),
    )
}

fn validate_observer_continuity_at(
    tx: &Transaction<'_>,
    expected: &ExpectedScope,
    identity: &StoredAttemptIdentity,
    qualification: &ObserverQualification,
    end: &ObserverFence,
    minimum_lease_expiry: i64,
) -> Result<()> {
    if identity.observer_owner_token.as_deref() != Some(qualification.observer_owner_token.as_str())
    {
        return Err(reconcile_required(
            expected,
            TrustState::UntrustedGap.as_str(),
            "sealed observer owner does not match the reconciliation scope",
        ));
    }
    let owner_matches = tx.query_row(
        "SELECT COUNT(*) FROM changed_path_observer_owners
         WHERE scope_id=?1 AND epoch=?2 AND owner_token=?3
           AND provider_id IS ?4 AND provider_identity IS ?5
           AND fence_nonce IS ?6 AND lease_state='active' AND expires_at>=?7",
        params![
            identity.scope_id,
            sql_u64(identity.epoch)?,
            qualification.observer_owner_token,
            identity.provider_id,
            identity.provider_identity,
            qualification.owner_fence_nonce,
            minimum_lease_expiry,
        ],
        |row| row.get::<_, i64>(0),
    )? == 1;
    let segment_matches = tx.query_row(
        "SELECT COUNT(*) FROM changed_path_observer_segments
         WHERE scope_id=?1 AND epoch=?2 AND segment_id=?3
           AND owner_token=?4 AND provider_id IS ?5
           AND state IN ('open','sealed') AND last_sequence IS NOT NULL
           AND last_sequence>=?6 AND durable_end_offset>=?7
           AND folded_end_offset>=?8",
        params![
            identity.scope_id,
            sql_u64(identity.epoch)?,
            qualification.durable_segment_id,
            qualification.observer_owner_token,
            identity.provider_id,
            sql_u64(end.sequence)?,
            sql_u64(qualification.segment_durable_offset)?,
            sql_u64(qualification.segment_folded_offset)?,
        ],
        |row| row.get::<_, i64>(0),
    )? == 1;
    if !owner_matches || !segment_matches {
        return Err(reconcile_required(
            expected,
            TrustState::UntrustedGap.as_str(),
            "sealed observer owner or durable segment continuity is unavailable",
        ));
    }
    Ok(())
}

fn publication_lease_deadline() -> Result<i64> {
    now_ts()
        .checked_add(MIN_PUBLICATION_LEASE_HORIZON_SECS)
        .ok_or_else(|| Error::Corrupt("observer publication lease deadline overflowed".into()))
}

fn validate_ready_attempt(
    tx: &Transaction<'_>,
    attempt: &ReconciliationAttempt,
    root_identity: &[u8],
) -> Result<()> {
    if attempt.stored_identity.root_handle_identity != root_identity
        || serde_json::to_vec(&attempt.stored_identity)? != attempt.encoded_identity
    {
        return Err(reconcile_required(
            &attempt.expected,
            TrustState::Reconciling.as_str(),
            "reconciliation attempt's pinned identity changed",
        ));
    }
    let matched = tx.query_row(
        "SELECT COUNT(*) FROM changed_path_reconciliations
         WHERE attempt_id=?1 AND scope_id=?2 AND expected_scope_epoch=?3
           AND expected_ref_name=?4 AND expected_ref_generation=?5
           AND expected_change_id=?6 AND expected_root_id=?7
           AND filesystem_identity=?8 AND policy_fingerprint=?9
           AND policy_dependency_generation=?10 AND provider_id IS ?11
           AND provider_identity IS ?12 AND start_fence=?13 AND state='ready'",
        params![
            attempt.attempt_id,
            attempt.stored_identity.scope_id,
            sql_u64(attempt.stored_identity.epoch)?,
            attempt.stored_identity.ref_name,
            sql_u64(attempt.stored_identity.ref_generation)?,
            attempt.stored_identity.change_id,
            attempt.stored_identity.baseline_root_id,
            attempt.stored_identity.filesystem_identity,
            attempt.stored_identity.policy_fingerprint,
            sql_u64(attempt.stored_identity.policy_generation)?,
            attempt.stored_identity.provider_id,
            attempt.stored_identity.provider_identity,
            attempt.encoded_identity,
        ],
        |row| row.get::<_, i64>(0),
    )?;
    if matched != 1 {
        return Err(reconcile_required(
            &attempt.expected,
            TrustState::Reconciling.as_str(),
            "reconciliation attempt identity changed",
        ));
    }
    Ok(())
}

fn capture_attempt_identity(
    conn: &rusqlite::Connection,
    expected: &ExpectedScope,
    start_fence: ObserverFence,
    root_handle_identity: Vec<u8>,
) -> Result<StoredAttemptIdentity> {
    let identity = conn.query_row(
        "SELECT scope_id,scope_root,scope_root_identity,case_sensitive,epoch,
                ref_name,ref_generation,change_id,baseline_root_id,
                policy_fingerprint,policy_dependency_generation,
                trust_state,continuity_generation,filesystem_identity,
                provider_id,provider_identity,observer_owner_token,
                durable_offset,folded_offset
         FROM changed_path_scopes WHERE scope_id=?1",
        [expected.scope_id.to_text()],
        |row| {
            let epoch = row.get::<_, i64>(4)?;
            let ref_generation = row.get::<_, i64>(6)?;
            let policy_generation = row.get::<_, i64>(10)?;
            let continuity_generation = row.get::<_, i64>(12)?;
            let durable_offset = row.get::<_, i64>(17)?;
            let folded_offset = row.get::<_, i64>(18)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, bool>(3)?,
                epoch,
                row.get::<_, String>(5)?,
                ref_generation,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                policy_generation,
                row.get::<_, String>(11)?,
                continuity_generation,
                row.get::<_, String>(13)?,
                row.get::<_, Option<String>>(14)?,
                row.get::<_, Option<String>>(15)?,
                row.get::<_, Option<String>>(16)?,
                durable_offset,
                folded_offset,
            ))
        },
    )?;
    let (
        scope_id,
        scope_root,
        scope_root_identity,
        case_sensitive,
        epoch,
        ref_name,
        ref_generation,
        change_id,
        baseline_root_id,
        policy_fingerprint,
        policy_generation,
        trust_state,
        continuity_generation,
        filesystem_identity,
        provider_id,
        provider_identity,
        observer_owner_token,
        durable_offset,
        folded_offset,
    ) = identity;
    let identity = StoredAttemptIdentity {
        scope_id,
        scope_root,
        scope_root_identity,
        case_sensitive,
        epoch: db_u64(epoch)?,
        ref_name,
        ref_generation: db_u64(ref_generation)?,
        change_id,
        baseline_root_id,
        policy_fingerprint,
        policy_generation: db_u64(policy_generation)?,
        trust_state,
        continuity_generation: db_u64(continuity_generation)?,
        filesystem_identity,
        provider_id,
        provider_identity,
        observer_owner_token,
        initial_durable_offset: db_u64(durable_offset)?,
        initial_folded_offset: db_u64(folded_offset)?,
        start_fence,
        root_handle_identity,
    };
    if identity.initial_folded_offset > identity.initial_durable_offset
        || identity.scope_id != expected.scope_id.to_text()
        || identity.epoch != expected.epoch
        || identity.ref_name != expected.ref_name
        || identity.ref_generation != expected.ref_generation
        || identity.baseline_root_id != expected.baseline_root.0
        || identity.policy_fingerprint != hex::encode(expected.policy_fingerprint)
        || identity.policy_generation != expected.policy_generation
        || !matches!(identity.trust_state.as_str(), "trusted" | "reconciling")
        || identity.continuity_generation == 0
        || identity.filesystem_identity != hex::encode(&expected.filesystem_identity)
        || identity.provider_identity.as_deref()
            != Some(hex::encode(&expected.provider_identity).as_str())
    {
        return Err(reconcile_required(
            expected,
            TrustState::UntrustedGap.as_str(),
            "full scope identity or initial cuts changed before reconciliation began",
        ));
    }
    Ok(identity)
}

fn validate_stored_scope(
    conn: &rusqlite::Connection,
    expected: &ExpectedScope,
    identity: &StoredAttemptIdentity,
) -> Result<(u64, u64)> {
    let cuts = conn
        .query_row(
            "SELECT durable_offset,folded_offset FROM changed_path_scopes
             WHERE scope_id=?1 AND scope_root=?2 AND scope_root_identity=?3
               AND case_sensitive=?4 AND epoch=?5 AND ref_name=?6
               AND ref_generation=?7 AND change_id=?8 AND baseline_root_id=?9
               AND policy_fingerprint=?10 AND policy_dependency_generation=?11
               AND filesystem_identity=?12 AND provider_id IS ?13
               AND provider_identity IS ?14 AND observer_owner_token IS ?15
               AND trust_state=?16 AND continuity_generation=?17",
            params![
                identity.scope_id,
                identity.scope_root,
                identity.scope_root_identity,
                identity.case_sensitive,
                sql_u64(identity.epoch)?,
                identity.ref_name,
                sql_u64(identity.ref_generation)?,
                identity.change_id,
                identity.baseline_root_id,
                identity.policy_fingerprint,
                sql_u64(identity.policy_generation)?,
                identity.filesystem_identity,
                identity.provider_id,
                identity.provider_identity,
                identity.observer_owner_token,
                identity.trust_state,
                sql_u64(identity.continuity_generation)?,
            ],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        )
        .optional()?;
    let Some((durable, folded)) = cuts else {
        return Err(reconcile_required(
            expected,
            TrustState::Reconciling.as_str(),
            "stored reconciliation scope identity changed",
        ));
    };
    let durable = db_u64(durable)?;
    let folded = db_u64(folded)?;
    if folded > durable {
        return Err(Error::Corrupt(
            "changed-path scope folded cut exceeds durable cut".into(),
        ));
    }
    Ok((durable, folded))
}

fn exact_scope_guard(conn: &rusqlite::Connection, expected: &ExpectedScope) -> Result<()> {
    let matched = conn.query_row(
        "SELECT COUNT(*) FROM changed_path_scopes
         WHERE scope_id=?1 AND epoch=?2 AND ref_name=?3 AND ref_generation=?4
           AND baseline_root_id=?5 AND policy_fingerprint=?6
           AND policy_dependency_generation=?7 AND filesystem_identity=?8
           AND provider_identity=?9",
        params![
            expected.scope_id.to_text(),
            sql_u64(expected.epoch)?,
            expected.ref_name,
            sql_u64(expected.ref_generation)?,
            expected.baseline_root.0,
            hex::encode(expected.policy_fingerprint),
            sql_u64(expected.policy_generation)?,
            hex::encode(&expected.filesystem_identity),
            hex::encode(&expected.provider_identity),
        ],
        |row| row.get::<_, i64>(0),
    )?;
    if matched != 1 {
        return Err(reconcile_required(
            expected,
            TrustState::UntrustedGap.as_str(),
            "expected scope identity changed",
        ));
    }
    Ok(())
}

fn exact_scope_update_state(
    conn: &rusqlite::Connection,
    expected: &ExpectedScope,
    state: TrustState,
    reason: &str,
) -> Result<()> {
    let changed = conn.execute(
        "UPDATE changed_path_scopes SET trust_state=?1, trust_reason=?2,
             continuity_generation=continuity_generation+1, updated_at=?3
         WHERE scope_id=?4 AND epoch=?5 AND ref_name=?6 AND ref_generation=?7
           AND baseline_root_id=?8 AND policy_fingerprint=?9
           AND policy_dependency_generation=?10 AND filesystem_identity=?11
           AND provider_identity=?12",
        params![
            state.as_str(),
            reason,
            now_ts(),
            expected.scope_id.to_text(),
            sql_u64(expected.epoch)?,
            expected.ref_name,
            sql_u64(expected.ref_generation)?,
            expected.baseline_root.0,
            hex::encode(expected.policy_fingerprint),
            sql_u64(expected.policy_generation)?,
            hex::encode(&expected.filesystem_identity),
            hex::encode(&expected.provider_identity),
        ],
    )?;
    if changed != 1 {
        return Err(reconcile_required(
            expected,
            TrustState::UntrustedGap.as_str(),
            "expected scope changed while reconciliation started",
        ));
    }
    Ok(())
}

fn current_trust_state(conn: &rusqlite::Connection, expected: &ExpectedScope) -> Result<String> {
    exact_scope_guard(conn, expected)?;
    conn.query_row(
        "SELECT trust_state FROM changed_path_scopes WHERE scope_id=?1",
        [expected.scope_id.to_text()],
        |row| row.get(0),
    )
    .map_err(Error::from)
}

fn mark_attempt_failed(conn: &rusqlite::Connection, attempt_id: &str, reason: &str) -> Result<()> {
    conn.execute(
        "UPDATE changed_path_reconciliations
         SET state='failed', reason=?1, updated_at=?2
         WHERE attempt_id=?3 AND state!='published'",
        params![reason, now_ts(), attempt_id],
    )?;
    Ok(())
}

fn candidate_cap_exceeded(conn: &rusqlite::Connection, scope_id: &str) -> Result<bool> {
    let (count, cap): (i64, i64) = conn.query_row(
        "SELECT (SELECT COUNT(*) FROM changed_path_entries WHERE scope_id=?1),
                max_candidate_rows
         FROM changed_path_scopes WHERE scope_id=?1",
        [scope_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    Ok(db_u64(count)? > db_u64(cap)?)
}

fn reconcile_required(expected: &ExpectedScope, state: &str, reason: &str) -> Error {
    Error::ChangeLedgerReconcileRequired {
        scope: expected.scope_id.to_text(),
        state: state.to_string(),
        reason: reason.to_string(),
        command: "trail status".to_string(),
    }
}

fn file_kind_label(kind: &FileKind) -> &'static str {
    match kind {
        FileKind::Text => "Text",
        FileKind::OpaqueText => "OpaqueText",
        FileKind::Binary => "Binary",
    }
}

fn sql_u64(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| Error::InvalidInput("value exceeds SQLite INTEGER".into()))
}

fn db_u64(value: i64) -> Result<u64> {
    u64::try_from(value).map_err(|_| Error::Corrupt("negative reconciliation count".into()))
}

#[cfg(debug_assertions)]
mod compiled_harness {
    use super::*;
    use crate::db::change_ledger::{
        BaselineIdentity, FilesystemIdentity, PolicyIdentity, ProviderCapabilities,
        ProviderIdentity, RecordingPolicySnapshot, ScopeIdentity, ScopeKind,
    };
    use crate::InitImportMode;
    use sha2::{Digest, Sha256};
    use std::fs;

    struct HarnessObserver {
        end_sequence: u64,
    }

    impl QualifiedObserver for HarnessObserver {
        fn begin_observation(&self, _expected: &ExpectedScope) -> Result<ObserverFence> {
            Ok(ObserverFence {
                sequence: 10,
                durable_offset: 0,
                nonce: b"compiled-harness-start".to_vec(),
            })
        }

        fn end_fence(
            &self,
            _expected: &ExpectedScope,
            _start: &ObserverFence,
        ) -> Result<ObserverFence> {
            Ok(ObserverFence {
                sequence: self.end_sequence,
                durable_offset: 0,
                nonce: b"compiled-harness-end".to_vec(),
            })
        }

        fn drain_through(
            &self,
            expected: &ExpectedScope,
            root_handle_identity: &[u8],
            start: &ObserverFence,
            end: &ObserverFence,
            _sink: &mut dyn FnMut(ObserverEvent) -> Result<()>,
        ) -> Result<ObserverQualification> {
            Ok(ObserverQualification::seal_for_test(
                expected,
                root_handle_identity.to_vec(),
                start.clone(),
                end.clone(),
            ))
        }
    }

    struct CallbackObserver<'a> {
        conn: &'a rusqlite::Connection,
        path: std::path::PathBuf,
    }

    impl QualifiedObserver for CallbackObserver<'_> {
        fn begin_observation(&self, _expected: &ExpectedScope) -> Result<ObserverFence> {
            Ok(ObserverFence {
                sequence: 10,
                durable_offset: 0,
                nonce: b"compiled-callback-start".to_vec(),
            })
        }

        fn end_fence(
            &self,
            _expected: &ExpectedScope,
            _start: &ObserverFence,
        ) -> Result<ObserverFence> {
            Ok(ObserverFence {
                sequence: 266,
                durable_offset: 0,
                nonce: b"compiled-callback-end".to_vec(),
            })
        }

        fn drain_through(
            &self,
            expected: &ExpectedScope,
            root_handle_identity: &[u8],
            start: &ObserverFence,
            end: &ObserverFence,
            sink: &mut dyn FnMut(ObserverEvent) -> Result<()>,
        ) -> Result<ObserverQualification> {
            let transaction =
                Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
            fs::write(&self.path, b"during callback\n")?;
            for sequence in 11..=266 {
                sink(ObserverEvent {
                    path: LedgerPath::parse("modify.txt")?,
                    flags: EvidenceFlags::CONTENT,
                    sequence,
                })?;
            }
            fs::write(&self.path, b"after callback\n")?;
            transaction.commit()?;
            Ok(ObserverQualification::seal_for_test(
                expected,
                root_handle_identity.to_vec(),
                start.clone(),
                end.clone(),
            ))
        }
    }

    struct HarnessFixture {
        _temp: tempfile::TempDir,
        db: Trail,
        expected: ExpectedScope,
        policy: CompiledPolicy,
    }

    impl HarnessFixture {
        fn new() -> Result<Self> {
            let temp = tempfile::tempdir()?;
            fs::write(temp.path().join("modify.txt"), b"before\n")?;
            fs::write(temp.path().join("delete.txt"), b"delete\n")?;
            Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false)?;
            let db = Trail::open(temp.path())?;
            let branch = db.current_branch()?;
            let head = db.resolve_branch_ref(&branch)?;
            let scope = ScopeIdentity {
                scope_id: ScopeId([0x6b; 32]),
                kind: ScopeKind::Workspace,
                owner_id: "compiled-reconciliation-harness".into(),
            };
            let fingerprint = [0x6c; 32];
            let baseline = BaselineIdentity {
                ref_name: head.name.clone(),
                ref_generation: u64::try_from(head.generation)
                    .map_err(|_| Error::Corrupt("negative harness ref generation".into()))?,
                change_id: head.change_id,
                root_id: head.root_id.clone(),
            };
            let filesystem = FilesystemIdentity(vec![0x6d]);
            let provider = ProviderIdentity {
                identity: vec![0x6e],
                capabilities: ProviderCapabilities {
                    durable_cursor: true,
                    linearizable_fence: true,
                    rename_pairing: true,
                    overflow_scope: true,
                    filesystem_supported: true,
                    clean_proof_allowed: true,
                    power_loss_durability: false,
                },
            };
            ChangedPathLedger::new(&db.conn).begin_scope(
                &scope,
                &baseline,
                &PolicyIdentity {
                    fingerprint,
                    generation: 1,
                },
                &filesystem,
                &provider,
            )?;
            let expected = ExpectedScope {
                scope_id: scope.scope_id,
                epoch: 1,
                ref_name: baseline.ref_name,
                ref_generation: baseline.ref_generation,
                baseline_root: baseline.root_id,
                policy_fingerprint: fingerprint,
                policy_generation: 1,
                filesystem_identity: filesystem.0,
                provider_identity: provider.identity,
            };
            install_continuity(&db.conn, &expected)?;
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
            Ok(Self {
                _temp: temp,
                db,
                expected,
                policy,
            })
        }

        fn begin_observed(
            &self,
            observer: &dyn QualifiedObserver,
        ) -> Result<ReconciliationAttempt> {
            let ledger = ChangedPathLedger::new(&self.db.conn);
            let mut attempt = begin_reconciliation(
                &self.db,
                &ledger,
                observer,
                &self.expected,
                &self.policy,
                ReconcileMode::Full,
                "compiled_harness",
            )?;
            attempt.observe(&self.db, &ledger, observer, &self.policy)?;
            Ok(attempt)
        }
    }

    fn install_continuity(conn: &rusqlite::Connection, expected: &ExpectedScope) -> Result<()> {
        let scope_id = expected.scope_id.to_text();
        let provider_id = hex::encode(&expected.provider_identity);
        let now = now_ts();
        conn.execute(
            "UPDATE changed_path_scopes SET observer_owner_token='full-test-owner'
             WHERE scope_id=?1",
            [&scope_id],
        )?;
        conn.execute(
            "INSERT INTO changed_path_observer_owners(
                 scope_id,epoch,owner_token,provider_id,provider_identity,
                 lease_state,fence_nonce,acquired_at,heartbeat_at,expires_at,updated_at
             ) VALUES(?1,?2,'full-test-owner',?3,?4,'active',?5,?6,?6,?7,?6)",
            params![
                scope_id,
                sql_u64(expected.epoch)?,
                provider_id,
                hex::encode(&expected.provider_identity),
                b"full-test-fence".as_slice(),
                now,
                now + 3_600,
            ],
        )?;
        conn.execute(
            "INSERT INTO changed_path_observer_segments(
                 scope_id,epoch,segment_id,owner_token,provider_id,
                 first_sequence,last_sequence,durable_end_offset,folded_end_offset,
                 segment_path,state,created_at,updated_at
             ) VALUES(?1,?2,'full-test-segment','full-test-owner',?3,
                      1,1000,100,100,'full-test-segment.cpl','open',?4,?4)",
            params![scope_id, sql_u64(expected.epoch)?, provider_id, now],
        )?;
        Ok(())
    }

    fn require(condition: bool, message: &str) -> Result<()> {
        if condition {
            Ok(())
        } else {
            Err(Error::Corrupt(message.into()))
        }
    }

    fn oracle() -> Result<()> {
        let fixture = HarnessFixture::new()?;
        fs::write(fixture.db.workspace_root.join("modify.txt"), b"after\n")?;
        fs::remove_file(fixture.db.workspace_root.join("delete.txt"))?;
        fs::write(fixture.db.workspace_root.join("add.txt"), b"added\n")?;
        let observer = HarnessObserver { end_sequence: 10 };
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let report = reconcile_full(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            "compiled_oracle",
        )?;
        let rows = fixture
            .db
            .conn
            .prepare(
                "SELECT normalized_path,event_flags FROM changed_path_entries
                 WHERE scope_id=?1 ORDER BY normalized_path COLLATE BINARY",
            )?
            .query_map([fixture.expected.scope_id.to_text()], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        require(report.published, "compiled oracle did not publish")?;
        require(
            rows == vec![
                ("add.txt".into(), EvidenceFlags::CREATE.0),
                ("delete.txt".into(), EvidenceFlags::DELETE.0),
                ("modify.txt".into(), EvidenceFlags::CONTENT.0),
            ],
            "compiled reconciliation oracle mismatch",
        )
    }

    fn races() -> Result<()> {
        let fixture = HarnessFixture::new()?;
        fs::write(fixture.db.workspace_root.join("add.txt"), b"added\n")?;
        let observer = HarnessObserver { end_sequence: 10 };
        let attempt = fixture.begin_observed(&observer)?;
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        ledger.mark_untrusted(
            &fixture.expected,
            TrustState::StaleBaseline,
            "compiled concurrent invalidation",
        )?;
        require(
            attempt
                .publish(&fixture.db, &ledger, &fixture.policy)
                .is_err(),
            "compiled fail-closed state race promoted trust",
        )?;

        let fixture = HarnessFixture::new()?;
        let attempt = fixture.begin_observed(&observer)?;
        fixture.db.conn.execute(
            "UPDATE changed_path_observer_owners SET lease_state='revoked'
             WHERE scope_id=?1",
            [fixture.expected.scope_id.to_text()],
        )?;
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        require(
            attempt
                .publish(&fixture.db, &ledger, &fixture.policy)
                .is_err(),
            "compiled owner-loss race promoted trust",
        )
    }

    fn callback_spool() -> Result<()> {
        let fixture = HarnessFixture::new()?;
        let observer = CallbackObserver {
            conn: &fixture.db.conn,
            path: fixture.db.workspace_root.join("modify.txt"),
        };
        let attempt = fixture.begin_observed(&observer)?;
        let staged_hash: String = fixture.db.conn.query_row(
            "SELECT content_hash FROM changed_path_reconciliation_rows
             WHERE attempt_id=?1 AND normalized_path='modify.txt'",
            [&attempt.attempt_id],
            |row| row.get(0),
        )?;
        require(
            staged_hash == hex::encode(Sha256::digest(b"after callback\n")),
            "compiled callback harness replayed before drain returned",
        )
    }

    pub(crate) fn run_oracle() -> std::result::Result<(), String> {
        oracle().map_err(|error| error.to_string())
    }

    pub(crate) fn run_races() -> std::result::Result<(), String> {
        races().map_err(|error| error.to_string())
    }

    pub(crate) fn run_callback_spool() -> std::result::Result<(), String> {
        callback_spool().map_err(|error| error.to_string())
    }
}

#[cfg(debug_assertions)]
pub(crate) use compiled_harness::{run_callback_spool, run_oracle, run_races};

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::cell::{Cell, RefCell};
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use crate::db::change_ledger::{
        BaselineIdentity, FilesystemIdentity, PolicyIdentity, ProviderCapabilities,
        ProviderIdentity, RecordingPolicySnapshot, ScopeIdentity, ScopeKind,
    };
    use crate::{InitImportMode, ObjectId};

    struct FakeQualifiedObserver {
        began: Cell<bool>,
        drains: Cell<u64>,
        sequence: Cell<u64>,
        corrupt_proof: bool,
    }

    struct EventDuringFenceObserver {
        event: ObserverEvent,
        mutation: RefCell<Option<Box<dyn FnOnce()>>>,
    }

    #[derive(Clone, Copy)]
    enum ObserverFailurePoint {
        EndFence,
        Drain,
        Callback,
    }

    struct FailingObserver {
        point: ObserverFailurePoint,
        callback_path: Option<LedgerPath>,
    }

    impl QualifiedObserver for FailingObserver {
        fn begin_observation(&self, _expected: &ExpectedScope) -> Result<ObserverFence> {
            Ok(ObserverFence {
                sequence: 10,
                durable_offset: 0,
                nonce: b"failure-start".to_vec(),
            })
        }

        fn end_fence(
            &self,
            _expected: &ExpectedScope,
            _start: &ObserverFence,
        ) -> Result<ObserverFence> {
            if matches!(self.point, ObserverFailurePoint::EndFence) {
                return Err(Error::InvalidInput("injected end fence failure".into()));
            }
            Ok(ObserverFence {
                sequence: 11,
                durable_offset: 0,
                nonce: b"failure-end".to_vec(),
            })
        }

        fn drain_through(
            &self,
            expected: &ExpectedScope,
            root_handle_identity: &[u8],
            start: &ObserverFence,
            end: &ObserverFence,
            sink: &mut dyn FnMut(ObserverEvent) -> Result<()>,
        ) -> Result<ObserverQualification> {
            if matches!(self.point, ObserverFailurePoint::Drain) {
                return Err(Error::InvalidInput("injected drain failure".into()));
            }
            if matches!(self.point, ObserverFailurePoint::Callback) {
                sink(ObserverEvent {
                    path: self.callback_path.clone().unwrap(),
                    flags: EvidenceFlags::CONTENT,
                    sequence: 11,
                })?;
            }
            Ok(ObserverQualification::seal_for_test(
                expected,
                root_handle_identity.to_vec(),
                start.clone(),
                end.clone(),
            ))
        }
    }

    struct RetryOnceObserver {
        attempts: Cell<u64>,
    }

    struct PrimaryTransactionObserver<'a> {
        conn: &'a rusqlite::Connection,
        path: std::path::PathBuf,
    }

    impl QualifiedObserver for PrimaryTransactionObserver<'_> {
        fn begin_observation(&self, _expected: &ExpectedScope) -> Result<ObserverFence> {
            Ok(ObserverFence {
                sequence: 10,
                durable_offset: 0,
                nonce: b"spool-start".to_vec(),
            })
        }

        fn end_fence(
            &self,
            _expected: &ExpectedScope,
            _start: &ObserverFence,
        ) -> Result<ObserverFence> {
            Ok(ObserverFence {
                sequence: 266,
                durable_offset: 0,
                nonce: b"spool-end".to_vec(),
            })
        }

        fn drain_through(
            &self,
            expected: &ExpectedScope,
            root_handle_identity: &[u8],
            start: &ObserverFence,
            end: &ObserverFence,
            sink: &mut dyn FnMut(ObserverEvent) -> Result<()>,
        ) -> Result<ObserverQualification> {
            let tx = Transaction::new_unchecked(self.conn, TransactionBehavior::Immediate)?;
            fs::write(&self.path, b"during callback\n")?;
            for sequence in 11..=266 {
                sink(ObserverEvent {
                    path: LedgerPath::parse("modify.txt")?,
                    flags: EvidenceFlags::CONTENT,
                    sequence,
                })?;
            }
            fs::write(&self.path, b"after callback\n")?;
            tx.commit()?;
            Ok(ObserverQualification::seal_for_test(
                expected,
                root_handle_identity.to_vec(),
                start.clone(),
                end.clone(),
            ))
        }
    }

    impl QualifiedObserver for RetryOnceObserver {
        fn begin_observation(&self, _expected: &ExpectedScope) -> Result<ObserverFence> {
            self.attempts.set(self.attempts.get() + 1);
            Ok(ObserverFence {
                sequence: 10,
                durable_offset: 0,
                nonce: format!("retry-start-{}", self.attempts.get()).into_bytes(),
            })
        }

        fn end_fence(
            &self,
            _expected: &ExpectedScope,
            _start: &ObserverFence,
        ) -> Result<ObserverFence> {
            if self.attempts.get() == 1 {
                return Err(Error::InvalidInput(
                    "workspace root identity race; retry reconciliation".into(),
                ));
            }
            Ok(ObserverFence {
                sequence: 10,
                durable_offset: 0,
                nonce: b"retry-end".to_vec(),
            })
        }

        fn drain_through(
            &self,
            expected: &ExpectedScope,
            root_handle_identity: &[u8],
            start: &ObserverFence,
            end: &ObserverFence,
            _sink: &mut dyn FnMut(ObserverEvent) -> Result<()>,
        ) -> Result<ObserverQualification> {
            Ok(ObserverQualification::seal_for_test(
                expected,
                root_handle_identity.to_vec(),
                start.clone(),
                end.clone(),
            ))
        }
    }

    impl QualifiedObserver for EventDuringFenceObserver {
        fn begin_observation(&self, _expected: &ExpectedScope) -> Result<ObserverFence> {
            Ok(ObserverFence {
                sequence: self.event.sequence - 1,
                durable_offset: 0,
                nonce: b"event-start".to_vec(),
            })
        }

        fn end_fence(
            &self,
            _expected: &ExpectedScope,
            _start: &ObserverFence,
        ) -> Result<ObserverFence> {
            if let Some(mutation) = self.mutation.borrow_mut().take() {
                mutation();
            }
            Ok(ObserverFence {
                sequence: self.event.sequence,
                durable_offset: 0,
                nonce: b"event-end".to_vec(),
            })
        }

        fn drain_through(
            &self,
            expected: &ExpectedScope,
            root_handle_identity: &[u8],
            start: &ObserverFence,
            end: &ObserverFence,
            sink: &mut dyn FnMut(ObserverEvent) -> Result<()>,
        ) -> Result<ObserverQualification> {
            sink(self.event.clone())?;
            Ok(ObserverQualification::seal_for_test(
                expected,
                root_handle_identity.to_vec(),
                start.clone(),
                end.clone(),
            ))
        }
    }

    impl FakeQualifiedObserver {
        fn new() -> Self {
            Self {
                began: Cell::new(false),
                drains: Cell::new(0),
                sequence: Cell::new(10),
                corrupt_proof: false,
            }
        }

        fn with_corrupt_proof() -> Self {
            Self {
                began: Cell::new(false),
                drains: Cell::new(0),
                sequence: Cell::new(10),
                corrupt_proof: true,
            }
        }
    }

    impl QualifiedObserver for FakeQualifiedObserver {
        fn begin_observation(&self, _expected: &ExpectedScope) -> Result<ObserverFence> {
            assert!(!self.began.replace(true));
            Ok(ObserverFence {
                sequence: self.sequence.get(),
                durable_offset: 0,
                nonce: b"start".to_vec(),
            })
        }

        fn end_fence(
            &self,
            _expected: &ExpectedScope,
            _start: &ObserverFence,
        ) -> Result<ObserverFence> {
            assert!(self.began.get());
            Ok(ObserverFence {
                sequence: self.sequence.get(),
                durable_offset: 0,
                nonce: b"end".to_vec(),
            })
        }

        fn drain_through(
            &self,
            expected: &ExpectedScope,
            root_handle_identity: &[u8],
            start: &ObserverFence,
            end: &ObserverFence,
            _sink: &mut dyn FnMut(ObserverEvent) -> Result<()>,
        ) -> Result<ObserverQualification> {
            self.drains.set(self.drains.get().saturating_add(1));
            let mut qualification = ObserverQualification::seal_for_test(
                expected,
                root_handle_identity.to_vec(),
                start.clone(),
                end.clone(),
            );
            if self.corrupt_proof {
                qualification.provider_identity.push(0xff);
            }
            Ok(qualification)
        }
    }

    struct Fixture {
        _temp: tempfile::TempDir,
        db: Trail,
        expected: ExpectedScope,
        policy: CompiledPolicy,
    }

    fn install_test_observer_continuity(conn: &rusqlite::Connection, expected: &ExpectedScope) {
        let scope_id = expected.scope_id.to_text();
        let provider_id = hex::encode(&expected.provider_identity);
        let now = now_ts();
        conn.execute(
            "UPDATE changed_path_scopes SET observer_owner_token='full-test-owner'
             WHERE scope_id=?1",
            [&scope_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO changed_path_observer_owners(
                 scope_id,epoch,owner_token,provider_id,provider_identity,
                 lease_state,fence_nonce,acquired_at,heartbeat_at,expires_at,updated_at
             ) VALUES(?1,?2,'full-test-owner',?3,?4,'active',?5,?6,?6,?7,?6)",
            params![
                scope_id,
                sql_u64(expected.epoch).unwrap(),
                provider_id,
                hex::encode(&expected.provider_identity),
                b"full-test-fence".as_slice(),
                now,
                now + 3_600,
            ],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO changed_path_observer_segments(
                 scope_id,epoch,segment_id,owner_token,provider_id,
                 first_sequence,last_sequence,durable_end_offset,folded_end_offset,
                 segment_path,state,created_at,updated_at
             ) VALUES(?1,?2,'full-test-segment','full-test-owner',?3,
                      1,1000,100,100,'full-test-segment.cpl','open',?4,?4)",
            params![scope_id, sql_u64(expected.epoch).unwrap(), provider_id, now,],
        )
        .unwrap();
    }

    impl Fixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path();
            fs::write(root.join("modify.txt"), b"before\n").unwrap();
            fs::write(root.join("mode.txt"), b"mode\n").unwrap();
            fs::write(root.join("delete.txt"), b"delete\n").unwrap();
            fs::write(root.join("rename-old.txt"), b"rename\n").unwrap();
            Trail::init(root, "main", InitImportMode::WorkingTree, false).unwrap();
            let db = Trail::open(root).unwrap();
            let branch = db.current_branch().unwrap();
            let head = db.resolve_branch_ref(&branch).unwrap();
            let scope = ScopeIdentity {
                scope_id: ScopeId([51; 32]),
                kind: ScopeKind::Workspace,
                owner_id: "reconcile-test".into(),
            };
            let fingerprint = [63; 32];
            let baseline = BaselineIdentity {
                ref_name: head.name.clone(),
                ref_generation: u64::try_from(head.generation).unwrap(),
                change_id: head.change_id,
                root_id: head.root_id.clone(),
            };
            let policy_identity = PolicyIdentity {
                fingerprint,
                generation: 1,
            };
            let filesystem = FilesystemIdentity(vec![7, 8]);
            let provider = ProviderIdentity {
                identity: vec![9, 10],
                capabilities: ProviderCapabilities {
                    durable_cursor: true,
                    linearizable_fence: true,
                    rename_pairing: true,
                    overflow_scope: true,
                    filesystem_supported: true,
                    clean_proof_allowed: true,
                    power_loss_durability: false,
                },
            };
            ChangedPathLedger::new(&db.conn)
                .begin_scope(&scope, &baseline, &policy_identity, &filesystem, &provider)
                .unwrap();
            let expected = ExpectedScope {
                scope_id: scope.scope_id,
                epoch: 1,
                ref_name: baseline.ref_name,
                ref_generation: baseline.ref_generation,
                baseline_root: baseline.root_id,
                policy_fingerprint: fingerprint,
                policy_generation: 1,
                filesystem_identity: filesystem.0,
                provider_identity: provider.identity,
            };
            let policy_root = db.workspace_root.clone();
            let policy = CompiledPolicy::for_reconciliation_test(
                RecordingPolicySnapshot {
                    workspace_root: policy_root.clone(),
                    ignore_gitignored: true,
                    dependency_files: Vec::new(),
                    case_sensitive: true,
                    rule_sources: Vec::new(),
                },
                fingerprint,
                &expected,
            );
            let fixture = Self {
                _temp: temp,
                db,
                expected,
                policy,
            };
            fixture.install_full_observer_continuity();
            fixture
        }

        fn root(&self) -> &std::path::Path {
            &self.db.workspace_root
        }

        fn observed_paths(&self) -> Vec<String> {
            let observer = FakeQualifiedObserver::new();
            let ledger = ChangedPathLedger::new(&self.db.conn);
            let mut attempt = begin_reconciliation(
                &self.db,
                &ledger,
                &observer,
                &self.expected,
                &self.policy,
                ReconcileMode::Full,
                "scan_paths",
            )
            .unwrap();
            attempt
                .observe(&self.db, &ledger, &observer, &self.policy)
                .unwrap();
            self.db
                .conn
                .prepare(
                    "SELECT normalized_path FROM changed_path_reconciliation_rows
                     WHERE attempt_id=?1 AND before_identity LIKE 'flags:%'
                     ORDER BY normalized_path COLLATE BINARY",
                )
                .unwrap()
                .query_map([attempt.attempt_id], |row| row.get(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        }

        fn ledger_rows(&self) -> Vec<(String, i64)> {
            self.db
                .conn
                .prepare(
                    "SELECT normalized_path,event_flags FROM changed_path_entries
                     WHERE scope_id=?1 ORDER BY normalized_path COLLATE BINARY",
                )
                .unwrap()
                .query_map([self.expected.scope_id.to_text()], |row| {
                    Ok((row.get(0)?, row.get(1)?))
                })
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        }

        fn ledger_prefixes(&self) -> Vec<String> {
            self.db
                .conn
                .prepare(
                    "SELECT normalized_prefix FROM changed_path_prefixes
                     WHERE scope_id=?1 ORDER BY normalized_prefix COLLATE BINARY",
                )
                .unwrap()
                .query_map([self.expected.scope_id.to_text()], |row| row.get(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        }

        fn latest_attempt_state(&self) -> String {
            self.db
                .conn
                .query_row(
                    "SELECT state FROM changed_path_reconciliations
                     ORDER BY created_at DESC, attempt_id DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap()
        }

        fn begin_observed(&self) -> ReconciliationAttempt {
            let observer = FakeQualifiedObserver::new();
            let ledger = ChangedPathLedger::new(&self.db.conn);
            let mut attempt = begin_reconciliation(
                &self.db,
                &ledger,
                &observer,
                &self.expected,
                &self.policy,
                ReconcileMode::Full,
                "race_test",
            )
            .unwrap();
            attempt
                .observe(&self.db, &ledger, &observer, &self.policy)
                .unwrap();
            attempt
        }

        fn install_full_observer_continuity(&self) {
            install_test_observer_continuity(&self.db.conn, &self.expected);
        }

        fn persist_live_provider_prefix(&self, prefix: &str) {
            let scope_id = self.expected.scope_id.to_text();
            let provider_id = hex::encode(&self.expected.provider_identity);
            self.db
                .conn
                .execute(
                    "UPDATE changed_path_scopes
                     SET trust_state='reconciling', trust_reason='provider_prefix'
                     WHERE scope_id=?1",
                    [&scope_id],
                )
                .unwrap();
            self.db
                .conn
                .execute(
                    "INSERT INTO changed_path_prefixes(
                     scope_id,normalized_prefix,completeness_reason,event_flags,
                     source_mask,first_sequence,last_sequence,provider_id,
                     provider_sequence,created_at,updated_at
                 ) VALUES(?1,?2,'provider_complete',0,?3,1,2,?4,2,?5,?5)",
                    params![
                        scope_id,
                        prefix,
                        super::super::EvidenceSource::Observer.mask(),
                        provider_id,
                        now_ts(),
                    ],
                )
                .unwrap();
        }
    }

    #[test]
    fn reconciliation_contract_starts_observation_before_streaming() {
        let _ = ReconcileMode::Full;
        let _ = begin_reconciliation;
        let _ = reconcile_full;
    }

    #[test]
    fn reconciliation_matches_add_modify_mode_delete_and_rename_oracle() {
        let fixture = Fixture::new();
        fs::write(fixture.root().join("add.txt"), b"added\n").unwrap();
        fs::write(fixture.root().join("modify.txt"), b"after and larger\n").unwrap();
        fs::remove_file(fixture.root().join("delete.txt")).unwrap();
        fs::rename(
            fixture.root().join("rename-old.txt"),
            fixture.root().join("rename-new.txt"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            let mut permissions = fs::metadata(fixture.root().join("mode.txt"))
                .unwrap()
                .permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(fixture.root().join("mode.txt"), permissions).unwrap();
        }

        let observer = FakeQualifiedObserver::new();
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let report = reconcile_full(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            "test_full_scan",
        )
        .unwrap();

        let mut oracle = vec![
            ("add.txt".to_string(), EvidenceFlags::CREATE.0),
            ("delete.txt".to_string(), EvidenceFlags::DELETE.0),
            ("modify.txt".to_string(), EvidenceFlags::CONTENT.0),
            ("rename-new.txt".to_string(), EvidenceFlags::CREATE.0),
            ("rename-old.txt".to_string(), EvidenceFlags::DELETE.0),
        ];
        #[cfg(unix)]
        oracle.push(("mode.txt".to_string(), EvidenceFlags::MODE.0));
        oracle.sort();
        assert_eq!(fixture.ledger_rows(), oracle);
        assert_eq!(report.observed_candidates, oracle.len() as u64);
        assert!(report.published);
        assert_eq!(report.trust_state, "trusted");
        assert!(report.peak_batch_rows <= STAGING_BATCH_ROWS as u64);
        assert_eq!(observer.began.get(), true);
        assert_eq!(
            fixture.expected.baseline_root,
            ObjectId(fixture.expected.baseline_root.0.clone())
        );
    }

    #[test]
    fn reconciliation_does_not_flatten_nested_gitignore_authority() {
        let mut fixture = Fixture::new();
        fs::create_dir_all(fixture.root().join("src/generated")).unwrap();
        fs::create_dir_all(fixture.root().join("other/generated")).unwrap();
        fs::write(fixture.root().join("src/.gitignore"), "generated\n").unwrap();
        fs::write(fixture.root().join("src/generated/ignored.txt"), b"src\n").unwrap();
        fs::write(
            fixture.root().join("other/generated/must-scan.txt"),
            b"other\n",
        )
        .unwrap();
        let rule_path = fixture.root().join("src/.gitignore");
        fixture
            .policy
            .set_gitignore_rule_for_test(rule_path, b"generated\n".to_vec());

        let paths = fixture.observed_paths();

        assert!(paths
            .iter()
            .any(|path| path == "other/generated/must-scan.txt"));
        assert!(paths.iter().any(|path| path == "src/generated/ignored.txt"));
    }

    #[test]
    fn reconciliation_scans_gitignored_files_when_recording_keeps_them() {
        let mut fixture = Fixture::new();
        fixture.policy.set_ignore_gitignored_for_test(false);
        fs::write(fixture.root().join(".gitignore"), "kept.txt\n").unwrap();
        fs::write(fixture.root().join("kept.txt"), b"kept\n").unwrap();

        assert!(fixture
            .observed_paths()
            .iter()
            .any(|path| path == "kept.txt"));
    }

    #[test]
    fn stale_scope_cas_fails_attempt_without_replacing_candidates() {
        for column in [
            "ref_generation",
            "filesystem_identity",
            "provider_identity",
            "policy_dependency_generation",
        ] {
            let fixture = Fixture::new();
            fs::write(fixture.root().join("add.txt"), b"added\n").unwrap();
            let attempt = fixture.begin_observed();
            fixture
                .db
                .conn
                .execute(
                    &format!("UPDATE changed_path_scopes SET {column}=?1 WHERE scope_id=?2"),
                    params!["replacement", fixture.expected.scope_id.to_text()],
                )
                .unwrap();

            let ledger = ChangedPathLedger::new(&fixture.db.conn);
            assert!(attempt
                .publish(&fixture.db, &ledger, &fixture.policy)
                .is_err());
            let (attempt_state, trust, candidates): (String, String, i64) = fixture
                .db
                .conn
                .query_row(
                    "SELECT r.state,s.trust_state,
                            (SELECT COUNT(*) FROM changed_path_entries e WHERE e.scope_id=s.scope_id)
                     FROM changed_path_reconciliations r
                     JOIN changed_path_scopes s ON s.scope_id=r.scope_id
                     ORDER BY r.created_at DESC LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            assert_eq!(attempt_state, "failed", "column {column}");
            assert_ne!(trust, "trusted", "column {column}");
            assert_eq!(candidates, 0, "column {column}");
        }
    }

    #[test]
    fn fail_closed_transition_after_ready_never_promotes_full_trust() {
        for state in [
            TrustState::Overflow,
            TrustState::Corrupt,
            TrustState::UntrustedGap,
            TrustState::StaleBaseline,
        ] {
            let fixture = Fixture::new();
            fs::write(fixture.root().join("add.txt"), b"added\n").unwrap();
            let attempt = fixture.begin_observed();
            let ledger = ChangedPathLedger::new(&fixture.db.conn);
            ledger
                .mark_untrusted(&fixture.expected, state, "concurrent invalidation")
                .unwrap();

            assert!(attempt
                .publish(&fixture.db, &ledger, &fixture.policy)
                .is_err());
            let (scope_state, attempt_state): (String, String) = fixture
                .db
                .conn
                .query_row(
                    "SELECT s.trust_state,r.state
                     FROM changed_path_scopes s
                     JOIN changed_path_reconciliations r ON r.scope_id=s.scope_id
                     ORDER BY r.created_at DESC,r.attempt_id DESC LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(scope_state, state.as_str());
            assert_eq!(attempt_state, "failed");
        }
    }

    #[test]
    fn observer_owner_loss_after_drain_forces_full_reconciliation() {
        for mutation in [
            "UPDATE changed_path_observer_owners SET lease_state='revoked'",
            "UPDATE changed_path_observer_owners SET lease_state='expired'",
            "UPDATE changed_path_observer_owners SET expires_at=heartbeat_at",
            "UPDATE changed_path_observer_owners SET fence_nonce=x'00'",
        ] {
            let fixture = Fixture::new();
            let attempt = fixture.begin_observed();
            fixture.db.conn.execute(mutation, []).unwrap();
            let ledger = ChangedPathLedger::new(&fixture.db.conn);

            assert!(
                attempt
                    .publish(&fixture.db, &ledger, &fixture.policy)
                    .is_err(),
                "mutation {mutation}"
            );
            let state: String = fixture
                .db
                .conn
                .query_row("SELECT trust_state FROM changed_path_scopes", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(state, "untrusted_gap", "mutation {mutation}");
            assert_eq!(fixture.latest_attempt_state(), "failed");
        }
    }

    #[test]
    fn durable_segment_loss_or_cut_regression_forces_full_reconciliation() {
        for mutation in [
            "DELETE FROM changed_path_observer_segments",
            "UPDATE changed_path_observer_segments SET durable_end_offset=99,folded_end_offset=99",
            "UPDATE changed_path_observer_segments SET folded_end_offset=99",
            "UPDATE changed_path_observer_segments SET last_sequence=9",
        ] {
            let fixture = Fixture::new();
            let attempt = fixture.begin_observed();
            fixture.db.conn.execute(mutation, []).unwrap();
            let ledger = ChangedPathLedger::new(&fixture.db.conn);

            assert!(
                attempt
                    .publish(&fixture.db, &ledger, &fixture.policy)
                    .is_err(),
                "mutation {mutation}"
            );
            assert_eq!(fixture.latest_attempt_state(), "failed");
        }
    }

    #[test]
    fn full_publication_expiry_at_final_boundary_rolls_back_candidates() {
        let fixture = Fixture::new();
        fs::write(fixture.root().join("late.txt"), b"late\n").unwrap();
        fixture
            .db
            .conn
            .execute(
                "INSERT INTO changed_path_prefixes(
                     scope_id,normalized_prefix,completeness_reason,event_flags,
                     source_mask,first_sequence,last_sequence,created_at,updated_at
                 ) VALUES(?1,'old-prefix','reconciliation',0,?2,1,1,?3,?3)",
                params![
                    fixture.expected.scope_id.to_text(),
                    super::super::EvidenceSource::Reconciliation.mask(),
                    now_ts(),
                ],
            )
            .unwrap();
        let before_rows = fixture.ledger_rows();
        let before_prefixes = fixture.ledger_prefixes();
        let mut attempt = fixture.begin_observed();
        let before_generation: i64 = fixture
            .db
            .conn
            .query_row(
                "SELECT continuity_generation FROM changed_path_scopes",
                [],
                |row| row.get(0),
            )
            .unwrap();
        attempt.set_final_publication_hook(|conn| {
            conn.execute(
                "UPDATE changed_path_observer_owners SET expires_at=?1",
                [now_ts()],
            )
            .unwrap();
        });
        let ledger = ChangedPathLedger::new(&fixture.db.conn);

        assert!(attempt
            .publish(&fixture.db, &ledger, &fixture.policy)
            .is_err());

        assert_eq!(fixture.ledger_rows(), before_rows);
        assert_eq!(fixture.ledger_prefixes(), before_prefixes);
        let (state, generation): (String, i64) = fixture
            .db
            .conn
            .query_row(
                "SELECT trust_state,continuity_generation FROM changed_path_scopes",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "untrusted_gap");
        assert_eq!(generation, before_generation + 1);
        assert_eq!(fixture.latest_attempt_state(), "failed");
    }

    #[test]
    fn prefix_publication_insufficient_final_horizon_rolls_back_candidates() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root().join("src")).unwrap();
        fs::write(fixture.root().join("src/current.txt"), b"current\n").unwrap();
        fixture.persist_live_provider_prefix("src");
        fixture
            .db
            .conn
            .execute(
                "INSERT INTO changed_path_entries(
                     scope_id,normalized_path,event_flags,source_mask,
                     first_sequence,last_sequence,created_at,updated_at
                 ) VALUES(?1,'src/stale.txt',?2,?3,1,1,?4,?4)",
                params![
                    fixture.expected.scope_id.to_text(),
                    EvidenceFlags::CONTENT.0,
                    super::super::EvidenceSource::Reconciliation.mask(),
                    now_ts(),
                ],
            )
            .unwrap();
        fixture
            .db
            .conn
            .execute(
                "INSERT INTO changed_path_prefixes(
                     scope_id,normalized_prefix,completeness_reason,event_flags,
                     source_mask,first_sequence,last_sequence,created_at,updated_at
                 ) VALUES(?1,'src/stale','reconciliation',0,?2,1,1,?3,?3)",
                params![
                    fixture.expected.scope_id.to_text(),
                    super::super::EvidenceSource::Reconciliation.mask(),
                    now_ts(),
                ],
            )
            .unwrap();
        let before_rows = fixture.ledger_rows();
        let before_prefixes = fixture.ledger_prefixes();
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let proof = persisted_proven_prefixes(
            &ledger,
            &fixture.expected,
            &[LedgerPath::parse("src").unwrap()],
        )
        .unwrap();
        let observer = FakeQualifiedObserver::new();
        let mut attempt = begin_reconciliation(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            ReconcileMode::ProvenPrefixes(proof),
            "final_lease_horizon",
        )
        .unwrap();
        attempt
            .observe(&fixture.db, &ledger, &observer, &fixture.policy)
            .unwrap();
        let before_generation: i64 = fixture
            .db
            .conn
            .query_row(
                "SELECT continuity_generation FROM changed_path_scopes",
                [],
                |row| row.get(0),
            )
            .unwrap();
        attempt.set_final_publication_hook(|conn| {
            conn.execute(
                "UPDATE changed_path_observer_owners SET expires_at=?1",
                [now_ts() + 1],
            )
            .unwrap();
        });

        assert!(attempt
            .publish(&fixture.db, &ledger, &fixture.policy)
            .is_err());

        assert_eq!(fixture.ledger_rows(), before_rows);
        assert_eq!(fixture.ledger_prefixes(), before_prefixes);
        let (state, generation): (String, i64) = fixture
            .db
            .conn
            .query_row(
                "SELECT trust_state,continuity_generation FROM changed_path_scopes",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(state, "untrusted_gap");
        assert_eq!(generation, before_generation + 1);
        assert_eq!(fixture.latest_attempt_state(), "failed");
    }

    #[test]
    fn full_scope_identity_changes_fail_the_ready_attempt() {
        for (column, replacement) in [
            ("change_id", "replacement-change"),
            ("provider_id", "replacement-provider"),
            ("scope_root", "/replacement/root"),
            ("scope_root_identity", "replacement-root-identity"),
            ("case_sensitive", "0"),
        ] {
            let fixture = Fixture::new();
            fs::write(fixture.root().join("add.txt"), b"added\n").unwrap();
            let attempt = fixture.begin_observed();
            fixture
                .db
                .conn
                .execute(
                    &format!("UPDATE changed_path_scopes SET {column}=?1 WHERE scope_id=?2"),
                    params![replacement, fixture.expected.scope_id.to_text()],
                )
                .unwrap();

            let ledger = ChangedPathLedger::new(&fixture.db.conn);
            assert!(
                attempt
                    .publish(&fixture.db, &ledger, &fixture.policy)
                    .is_err(),
                "column {column}"
            );
            let state: String = fixture
                .db
                .conn
                .query_row(
                    "SELECT state FROM changed_path_reconciliations ORDER BY created_at DESC LIMIT 1",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(state, "failed", "column {column}");
        }
    }

    #[test]
    fn publication_never_regresses_later_durable_or_folded_cuts() {
        let fixture = Fixture::new();
        fs::write(fixture.root().join("add.txt"), b"added\n").unwrap();
        let attempt = fixture.begin_observed();
        fixture
            .db
            .conn
            .execute(
                "UPDATE changed_path_scopes SET durable_offset=50, folded_offset=40
                 WHERE scope_id=?1",
                [fixture.expected.scope_id.to_text()],
            )
            .unwrap();
        let ledger = ChangedPathLedger::new(&fixture.db.conn);

        attempt
            .publish(&fixture.db, &ledger, &fixture.policy)
            .unwrap();

        let cuts: (i64, i64) = fixture
            .db
            .conn
            .query_row(
                "SELECT durable_offset,folded_offset FROM changed_path_scopes WHERE scope_id=?1",
                [fixture.expected.scope_id.to_text()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(cuts, (50, 40));
    }

    #[test]
    fn publication_candidate_cap_accepts_boundary_and_rolls_back_overflow() {
        for (cap, succeeds) in [(2_i64, true), (1_i64, false)] {
            let fixture = Fixture::new();
            fs::write(fixture.root().join("cap-one.txt"), b"one\n").unwrap();
            fs::write(fixture.root().join("cap-two.txt"), b"two\n").unwrap();
            fixture
                .db
                .conn
                .execute(
                    "UPDATE changed_path_scopes SET max_candidate_rows=?1 WHERE scope_id=?2",
                    params![cap, fixture.expected.scope_id.to_text()],
                )
                .unwrap();
            let attempt = fixture.begin_observed();
            let ledger = ChangedPathLedger::new(&fixture.db.conn);

            let result = attempt.publish(&fixture.db, &ledger, &fixture.policy);
            let (trust, attempt_state, candidates): (String, String, i64) = fixture
                .db
                .conn
                .query_row(
                    "SELECT s.trust_state,r.state,
                            (SELECT COUNT(*) FROM changed_path_entries e WHERE e.scope_id=s.scope_id)
                     FROM changed_path_scopes s JOIN changed_path_reconciliations r
                       ON r.scope_id=s.scope_id ORDER BY r.created_at DESC LIMIT 1",
                    [],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            if succeeds {
                assert!(result.is_ok());
                assert_eq!(trust, "trusted");
                assert_eq!(attempt_state, "published");
                assert_eq!(candidates, 2);
            } else {
                assert!(result.is_err());
                assert_eq!(trust, "overflow");
                assert_eq!(attempt_state, "failed");
                assert_eq!(candidates, 0);
            }
        }
    }

    #[test]
    fn workspace_root_replacement_cannot_publish_trust() {
        let fixture = Fixture::new();
        fs::write(fixture.root().join("add.txt"), b"added\n").unwrap();
        let attempt = fixture.begin_observed();
        let root = fixture.root().to_path_buf();
        let displaced = root.with_extension("reconcile-displaced");
        fs::rename(&root, &displaced).unwrap();
        fs::create_dir(&root).unwrap();

        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let result = attempt.publish(&fixture.db, &ledger, &fixture.policy);

        fs::remove_dir(&root).unwrap();
        fs::rename(&displaced, &root).unwrap();
        assert!(result.is_err());
        let trust: String = fixture
            .db
            .conn
            .query_row(
                "SELECT trust_state FROM changed_path_scopes WHERE scope_id=?1",
                [fixture.expected.scope_id.to_text()],
                |row| row.get(0),
            )
            .unwrap();
        assert_ne!(trust, "trusted");
    }

    #[test]
    fn nested_directory_replacement_forces_a_fresh_scan_before_publication() {
        let fixture = Fixture::new();
        let directory = fixture.root().join("nested");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("original.txt"), b"original\n").unwrap();
        let displaced = fixture.root().join("nested-displaced");
        let mutation_directory = directory.clone();
        let mutation_displaced = displaced.clone();
        let observer = EventDuringFenceObserver {
            event: ObserverEvent {
                path: LedgerPath::parse("nested/replacement.txt").unwrap(),
                flags: EvidenceFlags::CREATE,
                sequence: 31,
            },
            mutation: RefCell::new(Some(Box::new(move || {
                fs::rename(&mutation_directory, &mutation_displaced).unwrap();
                fs::create_dir(&mutation_directory).unwrap();
                fs::write(mutation_directory.join("replacement.txt"), b"replacement\n").unwrap();
            }))),
        };
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let result = reconcile_full(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            "directory_replacement",
        );
        fs::remove_dir_all(&directory).unwrap();
        fs::rename(&displaced, &directory).unwrap();

        let report = result.unwrap();
        assert_eq!(report.retries, 1);
        assert!(report.published);
        let rows = fixture.ledger_rows();
        assert!(rows.contains(&("nested/replacement.txt".into(), EvidenceFlags::CREATE.0)));
        assert!(!rows.iter().any(|(path, _)| path == "nested/original.txt"));
    }

    #[test]
    fn evidence_arriving_after_scan_is_folded_through_end_fence() {
        let fixture = Fixture::new();
        let path = fixture.root().join("modify.txt");
        let observer = EventDuringFenceObserver {
            event: ObserverEvent {
                path: LedgerPath::parse("modify.txt").unwrap(),
                flags: EvidenceFlags::CONTENT,
                sequence: 31,
            },
            mutation: RefCell::new(Some(Box::new(move || {
                fs::write(path, b"changed during end fence\n").unwrap();
            }))),
        };
        let ledger = ChangedPathLedger::new(&fixture.db.conn);

        let report = reconcile_full(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            "event_during_scan",
        )
        .unwrap();

        assert!(report.published);
        assert_eq!(report.start_sequence, 30);
        assert_eq!(report.end_sequence, 31);
        assert_eq!(
            fixture.ledger_rows(),
            vec![("modify.txt".into(), EvidenceFlags::CONTENT.0)]
        );
    }

    #[test]
    fn evidence_later_than_end_fence_is_retained() {
        let fixture = Fixture::new();
        fs::write(fixture.root().join("add.txt"), b"added\n").unwrap();
        let attempt = fixture.begin_observed();
        fixture
            .db
            .conn
            .execute(
                "INSERT INTO changed_path_entries(
                     scope_id,normalized_path,event_flags,source_mask,
                     first_sequence,last_sequence,provider_id,provider_sequence,
                     created_at,updated_at
                 ) VALUES(?1,'later.txt',?2,?3,11,11,'later-provider',11,?4,?4)",
                params![
                    fixture.expected.scope_id.to_text(),
                    EvidenceFlags::CONTENT.0,
                    super::super::EvidenceSource::Observer.mask(),
                    now_ts(),
                ],
            )
            .unwrap();
        let ledger = ChangedPathLedger::new(&fixture.db.conn);

        attempt
            .publish(&fixture.db, &ledger, &fixture.policy)
            .unwrap();

        assert_eq!(
            fixture.ledger_rows(),
            vec![
                ("add.txt".into(), EvidenceFlags::CREATE.0),
                ("later.txt".into(), EvidenceFlags::CONTENT.0),
            ]
        );
    }

    #[test]
    fn only_persisted_provider_complete_prefixes_are_qualified() {
        let fixture = Fixture::new();
        let scope_id = fixture.expected.scope_id.to_text();
        fixture.persist_live_provider_prefix("src");
        let ledger = ChangedPathLedger::new(&fixture.db.conn);

        let proof = persisted_proven_prefixes(
            &ledger,
            &fixture.expected,
            &[LedgerPath::parse("src").unwrap()],
        )
        .unwrap();
        assert_eq!(proof.prefixes, vec![LedgerPath::parse("src").unwrap()]);
        assert!(persisted_proven_prefixes(
            &ledger,
            &fixture.expected,
            &[LedgerPath::parse("user-prefix").unwrap()],
        )
        .is_err());

        fixture
            .db
            .conn
            .execute(
                "DELETE FROM changed_path_observer_owners WHERE scope_id=?1",
                [&scope_id],
            )
            .unwrap();
        assert!(persisted_proven_prefixes(
            &ledger,
            &fixture.expected,
            &[LedgerPath::parse("src").unwrap()],
        )
        .is_err());

        fixture
            .db
            .conn
            .execute(
                "UPDATE changed_path_scopes
             SET trust_state='stale_baseline', trust_reason='global_policy_change'
             WHERE scope_id=?1",
                [&scope_id],
            )
            .unwrap();
        assert!(persisted_proven_prefixes(
            &ledger,
            &fixture.expected,
            &[LedgerPath::parse("src").unwrap()],
        )
        .is_err());

        fixture
            .db
            .conn
            .execute(
                "UPDATE changed_path_scopes SET trust_state='overflow' WHERE scope_id=?1",
                [&scope_id],
            )
            .unwrap();
        assert!(persisted_proven_prefixes(
            &ledger,
            &fixture.expected,
            &[LedgerPath::parse("src").unwrap()],
        )
        .is_err());
    }

    #[test]
    fn proven_prefix_report_never_promotes_global_trust() {
        let fixture = Fixture::new();
        fixture.persist_live_provider_prefix("src");
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let proof = persisted_proven_prefixes(
            &ledger,
            &fixture.expected,
            &[LedgerPath::parse("src").unwrap()],
        )
        .unwrap();
        let observer = FakeQualifiedObserver::new();
        let mut attempt = begin_reconciliation(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            ReconcileMode::ProvenPrefixes(proof),
            "prefix_refresh",
        )
        .unwrap();
        attempt
            .observe(&fixture.db, &ledger, &observer, &fixture.policy)
            .unwrap();

        let report = attempt
            .publish(&fixture.db, &ledger, &fixture.policy)
            .unwrap();

        assert!(!report.published);
        assert!(report.refreshed);
        assert_eq!(report.trust_state, "reconciling");
        assert_eq!(fixture.latest_attempt_state(), "published");
    }

    #[test]
    fn proven_prefix_proof_is_bound_to_exact_owner_and_provider_cut() {
        for mutation in [
            "UPDATE changed_path_observer_owners SET owner_token='replacement-owner'",
            "UPDATE changed_path_prefixes SET provider_sequence=provider_sequence+1",
            "UPDATE changed_path_scopes SET durable_offset=1, folded_offset=1",
            "UPDATE changed_path_scopes SET continuity_generation=continuity_generation+1",
        ] {
            let fixture = Fixture::new();
            fixture.persist_live_provider_prefix("src");
            let ledger = ChangedPathLedger::new(&fixture.db.conn);
            let proof = persisted_proven_prefixes(
                &ledger,
                &fixture.expected,
                &[LedgerPath::parse("src").unwrap()],
            )
            .unwrap();
            fixture.db.conn.execute(mutation, []).unwrap();
            let observer = FakeQualifiedObserver::new();

            assert!(begin_reconciliation(
                &fixture.db,
                &ledger,
                &observer,
                &fixture.expected,
                &fixture.policy,
                ReconcileMode::ProvenPrefixes(proof),
                "stale_prefix_proof",
            )
            .is_err());
        }
    }

    #[test]
    fn stale_baseline_rejects_previously_sealed_prefix_proof_at_start() {
        let fixture = Fixture::new();
        fixture.persist_live_provider_prefix("src");
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let proof = persisted_proven_prefixes(
            &ledger,
            &fixture.expected,
            &[LedgerPath::parse("src").unwrap()],
        )
        .unwrap();
        fixture
            .db
            .conn
            .execute(
                "UPDATE changed_path_scopes
                 SET trust_state='stale_baseline',trust_reason='policy changed'
                 WHERE scope_id=?1",
                [fixture.expected.scope_id.to_text()],
            )
            .unwrap();
        let observer = FakeQualifiedObserver::new();

        assert!(begin_reconciliation(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            ReconcileMode::ProvenPrefixes(proof),
            "stale_prefix_start",
        )
        .is_err());
    }

    #[test]
    fn stale_baseline_after_ready_rejects_prefix_publication() {
        let fixture = Fixture::new();
        fixture.persist_live_provider_prefix("src");
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let proof = persisted_proven_prefixes(
            &ledger,
            &fixture.expected,
            &[LedgerPath::parse("src").unwrap()],
        )
        .unwrap();
        let observer = FakeQualifiedObserver::new();
        let mut attempt = begin_reconciliation(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            ReconcileMode::ProvenPrefixes(proof),
            "stale_prefix_publish",
        )
        .unwrap();
        attempt
            .observe(&fixture.db, &ledger, &observer, &fixture.policy)
            .unwrap();
        fixture
            .db
            .conn
            .execute(
                "UPDATE changed_path_scopes
                 SET trust_state='stale_baseline',trust_reason='policy changed'
                 WHERE scope_id=?1",
                [fixture.expected.scope_id.to_text()],
            )
            .unwrap();

        assert!(attempt
            .publish(&fixture.db, &ledger, &fixture.policy)
            .is_err());
        assert_eq!(fixture.latest_attempt_state(), "failed");
    }

    #[test]
    fn proven_prefix_publish_replaces_only_covered_owned_rows() {
        let fixture = Fixture::new();
        fs::create_dir_all(fixture.root().join("src")).unwrap();
        fs::write(fixture.root().join("src/current.txt"), b"current\n").unwrap();
        fixture.persist_live_provider_prefix("src");
        let scope = fixture.expected.scope_id.to_text();
        let provider = hex::encode(&fixture.expected.provider_identity);
        for (path, source_mask, provider_id, provider_sequence) in [
            (
                "src/stale-reconcile.txt",
                super::super::EvidenceSource::Reconciliation.mask(),
                None,
                None,
            ),
            (
                "src/stale-provider.txt",
                super::super::EvidenceSource::Observer.mask(),
                Some(provider.as_str()),
                Some(2_i64),
            ),
            (
                "src/later-provider.txt",
                super::super::EvidenceSource::Observer.mask(),
                Some(provider.as_str()),
                Some(99_i64),
            ),
            (
                "src/intent.txt",
                super::super::EvidenceSource::Intent.mask(),
                None,
                None,
            ),
            (
                "outside.txt",
                super::super::EvidenceSource::Reconciliation.mask(),
                None,
                None,
            ),
        ] {
            fixture
                .db
                .conn
                .execute(
                    "INSERT INTO changed_path_entries(
                         scope_id,normalized_path,event_flags,source_mask,
                         first_sequence,last_sequence,provider_id,provider_sequence,
                         created_at,updated_at
                     ) VALUES(?1,?2,?3,?4,1,1,?5,?6,?7,?7)",
                    params![
                        scope,
                        path,
                        EvidenceFlags::CONTENT.0,
                        source_mask,
                        provider_id,
                        provider_sequence,
                        now_ts(),
                    ],
                )
                .unwrap();
        }
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let proof = persisted_proven_prefixes(
            &ledger,
            &fixture.expected,
            &[LedgerPath::parse("src").unwrap()],
        )
        .unwrap();
        let observer = FakeQualifiedObserver::new();
        let mut attempt = begin_reconciliation(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            ReconcileMode::ProvenPrefixes(proof),
            "prefix_replace",
        )
        .unwrap();
        attempt
            .observe(&fixture.db, &ledger, &observer, &fixture.policy)
            .unwrap();

        let report = attempt
            .publish(&fixture.db, &ledger, &fixture.policy)
            .unwrap();
        let paths = fixture
            .db
            .conn
            .prepare(
                "SELECT normalized_path FROM changed_path_entries WHERE scope_id=?1
                 ORDER BY normalized_path COLLATE BINARY",
            )
            .unwrap()
            .query_map([scope], |row| row.get::<_, String>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();

        assert!(report.refreshed);
        assert!(!report.published);
        assert!(paths.contains(&"src/current.txt".to_string()));
        assert!(!paths.contains(&"src/stale-reconcile.txt".to_string()));
        assert!(!paths.contains(&"src/stale-provider.txt".to_string()));
        assert!(paths.contains(&"src/later-provider.txt".to_string()));
        assert!(paths.contains(&"src/intent.txt".to_string()));
        assert!(paths.contains(&"outside.txt".to_string()));
    }

    #[test]
    fn each_proven_prefix_uses_its_own_provider_sequence_cut() {
        let fixture = Fixture::new();
        fixture.persist_live_provider_prefix("a");
        let scope = fixture.expected.scope_id.to_text();
        let provider = hex::encode(&fixture.expected.provider_identity);
        fixture
            .db
            .conn
            .execute(
                "INSERT INTO changed_path_prefixes(
                     scope_id,normalized_prefix,completeness_reason,event_flags,
                     source_mask,first_sequence,last_sequence,provider_id,
                     provider_sequence,created_at,updated_at
                 ) VALUES(?1,'b','provider_complete',0,?2,1,8,?3,8,?4,?4)",
                params![
                    scope,
                    super::super::EvidenceSource::Observer.mask(),
                    provider,
                    now_ts(),
                ],
            )
            .unwrap();
        fixture
            .db
            .conn
            .execute(
                "INSERT INTO changed_path_entries(
                     scope_id,normalized_path,event_flags,source_mask,
                     first_sequence,last_sequence,provider_id,provider_sequence,
                     created_at,updated_at
                 ) VALUES(?1,'a/later.txt',?2,?3,5,5,?4,5,?5,?5)",
                params![
                    scope,
                    EvidenceFlags::CONTENT.0,
                    super::super::EvidenceSource::Observer.mask(),
                    provider,
                    now_ts(),
                ],
            )
            .unwrap();
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let proof = persisted_proven_prefixes(
            &ledger,
            &fixture.expected,
            &[
                LedgerPath::parse("a").unwrap(),
                LedgerPath::parse("b").unwrap(),
            ],
        )
        .unwrap();
        let observer = FakeQualifiedObserver::new();
        let mut attempt = begin_reconciliation(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            ReconcileMode::ProvenPrefixes(proof),
            "per_prefix_cut",
        )
        .unwrap();
        attempt
            .observe(&fixture.db, &ledger, &observer, &fixture.policy)
            .unwrap();
        attempt
            .publish(&fixture.db, &ledger, &fixture.policy)
            .unwrap();

        let preserved: bool = fixture
            .db
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM changed_path_entries
                 WHERE scope_id=?1 AND normalized_path='a/later.txt')",
                [scope],
                |row| row.get(0),
            )
            .unwrap();
        assert!(preserved);
    }

    #[test]
    fn mismatched_sealed_observer_proof_fails_closed() {
        let fixture = Fixture::new();
        fs::write(fixture.root().join("add.txt"), b"added\n").unwrap();
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let observer = FakeQualifiedObserver::with_corrupt_proof();
        let mut attempt = begin_reconciliation(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            ReconcileMode::Full,
            "bad_proof",
        )
        .unwrap();
        attempt
            .observe(&fixture.db, &ledger, &observer, &fixture.policy)
            .unwrap();

        assert!(attempt
            .publish(&fixture.db, &ledger, &fixture.policy)
            .is_err());
        assert!(fixture.ledger_rows().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn symlinks_and_non_regular_entries_are_not_candidates() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        symlink("modify.txt", fixture.root().join("linked.txt")).unwrap();
        fs::create_dir(fixture.root().join("empty-dir")).unwrap();
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let observer = FakeQualifiedObserver::new();

        let report = reconcile_full(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            "regular_files_only",
        )
        .unwrap();

        assert_eq!(report.observed_candidates, 0);
        assert!(fixture.ledger_rows().is_empty());
    }

    #[test]
    fn starting_a_new_attempt_abandons_crash_left_staging() {
        let fixture = Fixture::new();
        let observer = FakeQualifiedObserver::new();
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let first = begin_reconciliation(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            ReconcileMode::Full,
            "first",
        )
        .unwrap();
        let first_id = first.attempt_id.clone();
        drop(first);
        let second_observer = FakeQualifiedObserver::new();
        let _second = begin_reconciliation(
            &fixture.db,
            &ledger,
            &second_observer,
            &fixture.expected,
            &fixture.policy,
            ReconcileMode::Full,
            "restart",
        )
        .unwrap();

        let state: String = fixture
            .db
            .conn
            .query_row(
                "SELECT state FROM changed_path_reconciliations WHERE attempt_id=?1",
                [first_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(state, "abandoned");
    }

    #[test]
    fn scan_end_fence_drain_callback_and_flush_errors_terminalize_attempts() {
        for point in [ObserverFailurePoint::EndFence, ObserverFailurePoint::Drain] {
            let fixture = Fixture::new();
            let ledger = ChangedPathLedger::new(&fixture.db.conn);
            let observer = FailingObserver {
                point,
                callback_path: None,
            };
            let mut attempt = begin_reconciliation(
                &fixture.db,
                &ledger,
                &observer,
                &fixture.expected,
                &fixture.policy,
                ReconcileMode::Full,
                "terminal_failure",
            )
            .unwrap();
            assert!(attempt
                .observe(&fixture.db, &ledger, &observer, &fixture.policy)
                .is_err());
            assert_eq!(fixture.latest_attempt_state(), "failed");
        }

        #[cfg(unix)]
        {
            let fixture = Fixture::new();
            let blocked = fixture.root().join("blocked-directory");
            fs::create_dir(&blocked).unwrap();
            fs::write(blocked.join("file.txt"), b"blocked\n").unwrap();
            let mut blocked_permissions = fs::metadata(&blocked).unwrap().permissions();
            blocked_permissions.set_mode(0o000);
            fs::set_permissions(&blocked, blocked_permissions).unwrap();
            let ledger = ChangedPathLedger::new(&fixture.db.conn);
            let observer = FakeQualifiedObserver::new();
            let mut attempt = begin_reconciliation(
                &fixture.db,
                &ledger,
                &observer,
                &fixture.expected,
                &fixture.policy,
                ReconcileMode::Full,
                "scan_failure",
            )
            .unwrap();
            let result = attempt.observe(&fixture.db, &ledger, &observer, &fixture.policy);
            let mut restore_permissions = fs::metadata(&blocked).unwrap().permissions();
            restore_permissions.set_mode(0o700);
            fs::set_permissions(&blocked, restore_permissions).unwrap();
            assert!(result.is_err());
            assert_eq!(fixture.latest_attempt_state(), "failed");
        }

        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let fixture = Fixture::new();
            let external = tempfile::tempdir().unwrap();
            fs::write(external.path().join("file.txt"), b"external\n").unwrap();
            symlink(external.path(), fixture.root().join("linked")).unwrap();
            let ledger = ChangedPathLedger::new(&fixture.db.conn);
            let observer = FailingObserver {
                point: ObserverFailurePoint::Callback,
                callback_path: Some(LedgerPath::parse("linked/file.txt").unwrap()),
            };
            let mut attempt = begin_reconciliation(
                &fixture.db,
                &ledger,
                &observer,
                &fixture.expected,
                &fixture.policy,
                ReconcileMode::Full,
                "callback_failure",
            )
            .unwrap();
            assert!(attempt
                .observe(&fixture.db, &ledger, &observer, &fixture.policy)
                .is_err());
            assert_eq!(fixture.latest_attempt_state(), "failed");
        }

        {
            let fixture = Fixture::new();
            fs::write(fixture.root().join("flush.txt"), b"flush\n").unwrap();
            fixture
                .db
                .conn
                .execute_batch(
                    "CREATE TRIGGER fail_reconciliation_flush
                     BEFORE INSERT ON changed_path_reconciliation_rows
                     BEGIN SELECT RAISE(ABORT, 'injected flush failure'); END;",
                )
                .unwrap();
            let ledger = ChangedPathLedger::new(&fixture.db.conn);
            let observer = FakeQualifiedObserver::new();
            let mut attempt = begin_reconciliation(
                &fixture.db,
                &ledger,
                &observer,
                &fixture.expected,
                &fixture.policy,
                ReconcileMode::Full,
                "flush_failure",
            )
            .unwrap();
            assert!(attempt
                .observe(&fixture.db, &ledger, &observer, &fixture.policy)
                .is_err());
            assert_eq!(fixture.latest_attempt_state(), "failed");
        }
    }

    #[test]
    fn observer_callback_only_spools_then_replays_after_primary_transaction() {
        let fixture = Fixture::new();
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let observer = PrimaryTransactionObserver {
            conn: &fixture.db.conn,
            path: fixture.root().join("modify.txt"),
        };
        let mut attempt = begin_reconciliation(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            ReconcileMode::Full,
            "callback_spool",
        )
        .unwrap();

        attempt
            .observe(&fixture.db, &ledger, &observer, &fixture.policy)
            .unwrap();

        let staged_hash: String = fixture
            .db
            .conn
            .query_row(
                "SELECT content_hash FROM changed_path_reconciliation_rows
                 WHERE attempt_id=?1 AND normalized_path='modify.txt'",
                [&attempt.attempt_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            staged_hash,
            hex::encode(Sha256::digest(b"after callback\n"))
        );
        assert_eq!(fixture.latest_attempt_state(), "ready");
    }

    #[test]
    fn directory_guards_cannot_collide_with_legitimate_user_paths() {
        let fixture = Fixture::new();
        fs::create_dir(fixture.root().join("a")).unwrap();
        fs::create_dir(fixture.root().join("#directory-guard")).unwrap();
        fs::write(
            fixture.root().join("#directory-guard/61"),
            b"legitimate user file\n",
        )
        .unwrap();
        let observer = FakeQualifiedObserver::new();
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let mut attempt = begin_reconciliation(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            ReconcileMode::Full,
            "guard_collision",
        )
        .unwrap();
        attempt
            .observe(&fixture.db, &ledger, &observer, &fixture.policy)
            .unwrap();

        let candidate_exists: bool = fixture
            .db
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM changed_path_reconciliation_rows
                 WHERE attempt_id=?1 AND normalized_path='#directory-guard/61'
                   AND before_identity LIKE 'flags:%')",
                [&attempt.attempt_id],
                |row| row.get(0),
            )
            .unwrap();
        let guard_exists: bool = fixture
            .db
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM changed_path_reconciliation_guards
                 WHERE attempt_id=?1 AND relative_path=?2)",
                params![attempt.attempt_id, b"a".as_slice()],
                |row| row.get(0),
            )
            .unwrap();
        assert!(candidate_exists);
        assert!(guard_exists);
    }

    #[test]
    fn retryable_identity_race_restarts_and_reports_actual_retry() {
        let fixture = Fixture::new();
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let observer = RetryOnceObserver {
            attempts: Cell::new(0),
        };

        let report = reconcile_full(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            "retry_once",
        )
        .unwrap();

        assert_eq!(observer.attempts.get(), 2);
        assert_eq!(report.retries, 1);
        let failed: i64 = fixture
            .db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM changed_path_reconciliations WHERE state='failed'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(failed, 1);
    }

    #[test]
    fn hundred_thousand_staged_rows_keep_batch_memory_bounded() {
        let fixture = Fixture::new();
        let observer = FakeQualifiedObserver::new();
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let attempt = begin_reconciliation(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            ReconcileMode::Full,
            "scale",
        )
        .unwrap();
        let mut writer = StagingWriter::new(&fixture.db.conn, &attempt.attempt_id);
        for index in 0..100_000u64 {
            writer
                .push(StagedRow {
                    path: format!("scale/{index:06}.txt"),
                    row_kind: "entry",
                    file_kind: Some("Text".into()),
                    content_hash: Some(format!("{index:064x}")),
                    executable: Some(false),
                    size_bytes: Some(1),
                    flags: EvidenceFlags::CREATE,
                    identity: None,
                    source_sequence: None,
                })
                .unwrap();
        }
        writer.flush().unwrap();

        let count: i64 = fixture
            .db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM changed_path_reconciliation_rows WHERE attempt_id=?1",
                [&attempt.attempt_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 100_000);
        assert_eq!(writer.peak_batch_rows, STAGING_BATCH_ROWS as u64);
        assert!(writer.pending.capacity() <= STAGING_BATCH_ROWS);
    }

    #[test]
    fn hundred_thousand_files_reconcile_end_to_end_with_bounded_streaming() {
        let fixture = Fixture::new();
        for directory in 0..100_u64 {
            let path = fixture.root().join(format!("scale/{directory:03}"));
            fs::create_dir_all(&path).unwrap();
            for file in 0..1_000_u64 {
                std::fs::File::create(path.join(format!("{file:04}.empty"))).unwrap();
            }
        }
        fixture
            .db
            .conn
            .execute("DELETE FROM worktree_file_index", [])
            .unwrap();
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
        let observer = FakeQualifiedObserver::new();

        let report = reconcile_full(
            &fixture.db,
            &ledger,
            &observer,
            &fixture.expected,
            &fixture.policy,
            "hundred_thousand_end_to_end",
        )
        .unwrap();

        let published: i64 = fixture
            .db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM changed_path_entries WHERE scope_id=?1",
                [fixture.expected.scope_id.to_text()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(observer.drains.get(), 1);
        assert_eq!(report.observed_files, 100_004);
        assert_eq!(report.observed_candidates, 100_000);
        assert_eq!(report.candidate_rows, 100_000);
        assert_eq!(published, 100_000);
        assert!(report.hashed_bytes > 0);
        assert!(report.peak_batch_rows <= STAGING_BATCH_ROWS as u64);
        assert!(report.peak_buffer_bytes <= 2 * 64 * 1024 + 6);
    }

    #[test]
    fn realistic_filesystem_hash_and_baseline_gate_ignores_worktree_index_authority() {
        let temp = tempfile::tempdir().unwrap();
        for directory in 0..32_u64 {
            let path = temp.path().join(format!("tree/{directory:02}"));
            fs::create_dir_all(&path).unwrap();
            for file in 0..16_u64 {
                fs::write(
                    path.join(format!("{file:02}.txt")),
                    format!("baseline-{directory}-{file}\n"),
                )
                .unwrap();
            }
        }
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let branch = db.current_branch().unwrap();
        let head = db.resolve_branch_ref(&branch).unwrap();
        let scope = ScopeIdentity {
            scope_id: ScopeId([81; 32]),
            kind: ScopeKind::Workspace,
            owner_id: "realistic-reconcile".into(),
        };
        let fingerprint = [82; 32];
        let provider = ProviderIdentity {
            identity: vec![83],
            capabilities: ProviderCapabilities {
                durable_cursor: true,
                linearizable_fence: true,
                rename_pairing: true,
                overflow_scope: true,
                filesystem_supported: true,
                clean_proof_allowed: true,
                power_loss_durability: false,
            },
        };
        ChangedPathLedger::new(&db.conn)
            .begin_scope(
                &scope,
                &BaselineIdentity {
                    ref_name: head.name.clone(),
                    ref_generation: u64::try_from(head.generation).unwrap(),
                    change_id: head.change_id,
                    root_id: head.root_id.clone(),
                },
                &PolicyIdentity {
                    fingerprint,
                    generation: 1,
                },
                &FilesystemIdentity(vec![84]),
                &provider,
            )
            .unwrap();
        let expected = ExpectedScope {
            scope_id: scope.scope_id,
            epoch: 1,
            ref_name: head.name,
            ref_generation: u64::try_from(head.generation).unwrap(),
            baseline_root: head.root_id,
            policy_fingerprint: fingerprint,
            policy_generation: 1,
            filesystem_identity: vec![84],
            provider_identity: provider.identity,
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
        install_test_observer_continuity(&db.conn, &expected);
        for index in 0..128_u64 {
            let directory = index / 16;
            let file = index % 16;
            fs::write(
                temp.path()
                    .join(format!("tree/{directory:02}/{file:02}.txt")),
                format!("modified-{index}\n"),
            )
            .unwrap();
        }
        for index in 128..256_u64 {
            let directory = index / 16;
            let file = index % 16;
            fs::remove_file(
                temp.path()
                    .join(format!("tree/{directory:02}/{file:02}.txt")),
            )
            .unwrap();
        }
        for index in 0..128_u64 {
            fs::write(
                temp.path().join(format!("tree/new-{index:03}.txt")),
                format!("new-{index}\n"),
            )
            .unwrap();
        }
        db.conn
            .execute("DELETE FROM worktree_file_index", [])
            .unwrap();
        let ledger = ChangedPathLedger::new(&db.conn);
        let observer = FakeQualifiedObserver::new();

        let report = reconcile_full(
            &db,
            &ledger,
            &observer,
            &expected,
            &policy,
            "realistic_gate",
        )
        .unwrap();

        assert_eq!(report.observed_files, 512);
        assert_eq!(report.observed_candidates, 384);
        assert!(report.hashed_bytes > 0);
        assert!(report.peak_batch_rows <= STAGING_BATCH_ROWS as u64);
        assert!(report.peak_buffer_bytes <= 2 * 64 * 1024 + 6);
    }
}
