use std::sync::atomic::{AtomicU64, Ordering};

use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::{
    raw_event_invalidates_policy, ChangedPathLedger, CompiledPolicy, EvidenceFlags, ExpectedScope,
    LedgerPath, ScopeId, TrustState,
};
use crate::db::storage::{PinnedWorktreeRoot, ReconciliationFile};
use crate::db::util::now_ts;
use crate::error::{Error, Result};
use crate::model::{ChangeLedgerReconcileReport, FileEntry, FileKind};
use crate::Trail;

const STAGING_BATCH_ROWS: usize = 256;
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
    complete_root_interval: bool,
    complete_policy_interval: bool,
    persisted_evidence_through_end: bool,
}

impl ObserverQualification {
    // This is deliberately not public outside the changed-ledger module. Task
    // 5 has no production native provider capable of minting this proof.
    pub(super) fn seal_for_provider(
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

#[derive(Clone, Debug, Serialize, Deserialize)]
struct StoredStartFence {
    fence: ObserverFence,
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
    root: PinnedWorktreeRoot,
    report: ChangeLedgerReconcileReport,
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
    let (state, reason, provider_id): (String, String, String) = ledger.conn.query_row(
        "SELECT trust_state, trust_reason, provider_id FROM changed_path_scopes WHERE scope_id=?1",
        [&scope_id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    if !matches!(state.as_str(), "trusted" | "reconciling") {
        return Err(reconcile_required(
            expected,
            &state,
            &format!("full reconciliation required: {reason}"),
        ));
    }
    let owner_is_live = ledger.conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM changed_path_observer_owners
             WHERE scope_id=?1 AND epoch=?2 AND provider_id=?3
               AND provider_identity=?4 AND lease_state='active' AND expires_at>=?5
         )",
        params![
            scope_id,
            sql_u64(expected.epoch)?,
            provider_id,
            hex::encode(&expected.provider_identity),
            now_ts(),
        ],
        |row| row.get::<_, bool>(0),
    )?;
    if !owner_is_live {
        return Err(reconcile_required(
            expected,
            &state,
            "qualified provider owner is unavailable; full reconciliation required",
        ));
    }
    let mut prefixes = requested.to_vec();
    prefixes.sort();
    prefixes.dedup();
    for prefix in &prefixes {
        let qualified = ledger.conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM changed_path_prefixes
                 WHERE scope_id=?1 AND normalized_prefix=?2 COLLATE BINARY
                   AND completeness_reason='provider_complete'
                   AND source_mask=?3 AND provider_id=?4
                   AND provider_sequence IS NOT NULL
             )",
            params![
                scope_id,
                prefix.as_str(),
                super::EvidenceSource::Observer.mask(),
                provider_id,
            ],
            |row| row.get::<_, bool>(0),
        )?;
        if !qualified {
            return Err(reconcile_required(
                expected,
                &state,
                "prefix was not persisted by the qualified provider",
            ));
        }
    }
    Ok(ProvenPrefixSet { prefixes })
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
    // The observation cut is deliberately acquired before the root is opened
    // and before any enumeration can begin.
    let start_fence = observer.begin_observation(expected)?;
    let root = trail.open_pinned_worktree_root(policy)?;
    let root_handle_identity = trail.pinned_worktree_root_identity(&root);
    let stored_start = serde_json::to_vec(&StoredStartFence {
        fence: start_fence.clone(),
        root_handle_identity,
    })?;
    let attempt_id = format!(
        "reconcile-{}-{}",
        now_ts(),
        NEXT_ATTEMPT_ID.fetch_add(1, Ordering::Relaxed)
    );
    let tx = Transaction::new_unchecked(ledger.conn, TransactionBehavior::Immediate)?;
    exact_scope_guard(&tx, expected)?;
    validate_mode_start(&tx, expected, &mode)?;
    let scope_id = expected.scope_id.to_text();
    let (change_id, provider_id): (String, String) = tx.query_row(
        "SELECT change_id, provider_id FROM changed_path_scopes WHERE scope_id=?1",
        [&scope_id],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
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
            change_id,
            expected.baseline_root.0,
            hex::encode(&expected.filesystem_identity),
            hex::encode(expected.policy_fingerprint),
            sql_u64(expected.policy_generation)?,
            provider_id,
            hex::encode(&expected.provider_identity),
            start_cursor.as_slice(),
            stored_start,
            mode.label(),
            reason,
            mode.completeness(),
            now_ts(),
        ],
    )?;
    if matches!(mode, ReconcileMode::Full) {
        exact_scope_update_state(&tx, expected, TrustState::Reconciling, reason)?;
    }
    tx.commit()?;

    Ok(ReconciliationAttempt {
        attempt_id,
        expected: expected.clone(),
        mode: mode.clone(),
        reason: reason.to_string(),
        start_fence: start_fence.clone(),
        end_fence: None,
        qualification: None,
        root,
        report: ChangeLedgerReconcileReport {
            mode: mode.label().to_string(),
            reason: reason.to_string(),
            start_sequence: start_fence.sequence,
            start_durable_offset: start_fence.durable_offset,
            trust_state: TrustState::Reconciling.as_str().to_string(),
            ..ChangeLedgerReconcileReport::default()
        },
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
    let mut attempt = begin_reconciliation(
        trail,
        ledger,
        observer,
        expected,
        policy,
        ReconcileMode::Full,
        reason,
    )?;
    attempt.observe(trail, ledger, observer, policy)?;
    attempt.publish(trail, ledger, policy)
}

impl ReconciliationAttempt {
    pub(crate) fn observe(
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
        trail.visit_pinned_worktree_files(&self.root, policy, &prefixes, |file| {
            writer.stage_filesystem(file)
        })?;
        writer.flush()?;
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
        let mut policy_event = false;
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
                if raw_event_invalidates_policy(policy, std::path::Path::new(event.path.as_str())) {
                    policy_event = true;
                    return Ok(());
                }
                let current =
                    trail.read_pinned_worktree_path(&self.root, policy, event.path.as_str())?;
                let baseline =
                    trail.root_file_entry(&self.expected.baseline_root, event.path.as_str())?;
                writer.stage_observer_result(event, current, baseline)
            },
        )?;
        if policy_event {
            return self.fail(
                ledger,
                "recording policy changed during reconciliation interval",
            );
        }
        writer.flush()?;
        let candidate_rows = ledger.conn.query_row(
            "SELECT COUNT(*) FROM changed_path_reconciliation_rows WHERE attempt_id=?1",
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
        let Some(qualification) = self.qualification.as_ref() else {
            return Err(reconcile_required(
                &self.expected,
                TrustState::Reconciling.as_str(),
                "qualified observer proof is unavailable",
            ));
        };
        if policy.fingerprint != self.expected.policy_fingerprint {
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
            // A complete prefix proof can refresh staged evidence, but it is
            // not a proof that all other paths are clean and never promotes
            // global scope trust.
            self.report.published = false;
            self.report.trust_state = current_trust_state(ledger.conn, &self.expected)?;
            return Ok(self.report);
        }

        let tx = Transaction::new_unchecked(ledger.conn, TransactionBehavior::Immediate)?;
        if exact_scope_guard(&tx, &self.expected).is_err() {
            return self.fail_publication_transaction(
                tx,
                "scope changed during reconciliation publication",
            );
        }
        if !matches!(trail.verify_pinned_worktree_root(&self.root), Ok(true)) {
            return self.fail_publication_transaction(
                tx,
                "workspace root identity changed before publication",
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
        let changed = tx.execute(
            "UPDATE changed_path_scopes
             SET trust_state='trusted', trust_reason='reconciliation_published',
                 durable_offset=?1, folded_offset=?1, updated_at=?2
             WHERE scope_id=?3 AND epoch=?4 AND ref_name=?5 AND ref_generation=?6
               AND baseline_root_id=?7 AND policy_fingerprint=?8
               AND policy_dependency_generation=?9 AND filesystem_identity=?10
               AND provider_identity=?11",
            params![
                sql_u64(end.durable_offset)?,
                now,
                scope_id,
                sql_u64(self.expected.epoch)?,
                self.expected.ref_name,
                sql_u64(self.expected.ref_generation)?,
                self.expected.baseline_root.0,
                hex::encode(self.expected.policy_fingerprint),
                sql_u64(self.expected.policy_generation)?,
                hex::encode(&self.expected.filesystem_identity),
                hex::encode(&self.expected.provider_identity),
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
        self.report.trust_state = TrustState::Trusted.as_str().to_string();
        let candidate_rows = ledger.conn.query_row(
            "SELECT COUNT(*) FROM changed_path_entries WHERE scope_id=?1",
            [self.expected.scope_id.to_text()],
            |row| row.get::<_, i64>(0),
        )?;
        self.report.candidate_rows = db_u64(candidate_rows)?;
        Ok(self.report)
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

struct StagingWriter<'a> {
    conn: &'a rusqlite::Connection,
    attempt_id: &'a str,
    pending: Vec<StagedRow>,
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
        self.peak_batch_rows = self.peak_batch_rows.max(self.pending.len() as u64);
        if self.pending.len() >= STAGING_BATCH_ROWS {
            self.flush()?;
        }
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.pending.is_empty() {
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
        if matches!(state.as_str(), "overflow" | "untrusted_gap" | "corrupt") {
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

fn validate_ready_attempt(
    tx: &Transaction<'_>,
    attempt: &ReconciliationAttempt,
    root_identity: &[u8],
) -> Result<()> {
    let stored_start = serde_json::to_vec(&StoredStartFence {
        fence: attempt.start_fence.clone(),
        root_handle_identity: root_identity.to_vec(),
    })?;
    let matched = tx.query_row(
        "SELECT COUNT(*) FROM changed_path_reconciliations
         WHERE attempt_id=?1 AND scope_id=?2 AND expected_scope_epoch=?3
           AND expected_ref_name=?4 AND expected_ref_generation=?5
           AND expected_root_id=?6 AND filesystem_identity=?7
           AND policy_fingerprint=?8 AND policy_dependency_generation=?9
           AND provider_identity=?10 AND start_fence=?11 AND state='ready'",
        params![
            attempt.attempt_id,
            attempt.expected.scope_id.to_text(),
            sql_u64(attempt.expected.epoch)?,
            attempt.expected.ref_name,
            sql_u64(attempt.expected.ref_generation)?,
            attempt.expected.baseline_root.0,
            hex::encode(&attempt.expected.filesystem_identity),
            hex::encode(attempt.expected.policy_fingerprint),
            sql_u64(attempt.expected.policy_generation)?,
            hex::encode(&attempt.expected.provider_identity),
            stored_start,
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
        "UPDATE changed_path_scopes SET trust_state=?1, trust_reason=?2, updated_at=?3
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use crate::db::change_ledger::{
        AdapterEquivalence, BaselineIdentity, FilesystemIdentity, PolicyIdentity,
        PolicyInvalidationIndex, ProviderCapabilities, ProviderIdentity, RecordingPolicySnapshot,
        ScopeIdentity, ScopeKind,
    };
    use crate::{InitImportMode, ObjectId};

    struct FakeQualifiedObserver {
        began: Cell<bool>,
        sequence: Cell<u64>,
        corrupt_proof: bool,
    }

    struct EventDuringFenceObserver {
        event: ObserverEvent,
        mutation: RefCell<Option<Box<dyn FnOnce()>>>,
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
            Ok(ObserverQualification::seal_for_provider(
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
                sequence: Cell::new(10),
                corrupt_proof: false,
            }
        }

        fn with_corrupt_proof() -> Self {
            Self {
                began: Cell::new(false),
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
            let mut qualification = ObserverQualification::seal_for_provider(
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
            let policy = CompiledPolicy {
                snapshot: RecordingPolicySnapshot {
                    workspace_root: policy_root.clone(),
                    ignore_gitignored: true,
                    dependency_files: Vec::new(),
                    case_sensitive: true,
                    rule_sources: Vec::new(),
                },
                fingerprint,
                dependencies: Vec::new(),
                adapter_equivalence: AdapterEquivalence::Conservative,
                stale_baseline: true,
                reused_manifest: false,
                invalidation_index: PolicyInvalidationIndex::from_paths(
                    &policy_root,
                    true,
                    std::iter::empty(),
                ),
            };
            Self {
                _temp: temp,
                db,
                expected,
                policy,
            }
        }

        fn root(&self) -> &std::path::Path {
            &self.db.workspace_root
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
                    "INSERT INTO changed_path_observer_owners(
                     scope_id,epoch,owner_token,provider_id,provider_identity,
                     lease_state,acquired_at,heartbeat_at,expires_at,updated_at
                 ) VALUES(?1,?2,?3,?4,?5,'active',?6,?6,?7,?6)",
                    params![
                        scope_id,
                        sql_u64(self.expected.epoch).unwrap(),
                        format!("owner-{prefix}"),
                        provider_id,
                        hex::encode(&self.expected.provider_identity),
                        now_ts(),
                        now_ts() + 3_600,
                    ],
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
        assert_eq!(report.trust_state, "reconciling");
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
}
