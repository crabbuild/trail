use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use super::intent::{
    authoritative_ref_matches_target, db_u64, durable_intent_barrier, load_intent, sql_u64,
    stage_intent_evidence, validate_qualified_filesystem_proof, IntentId, IntentState,
    PersistedIntent,
};
use super::{ChangedPathLedger, ExpectedScope};
use crate::db::util::now_ts;
use crate::error::{Error, Result};
use crate::ObjectId;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RecoveryDecision {
    FinishPublication,
    RetainCandidatesAndAcknowledge,
    Abort,
    FullReconciliation,
}

pub(crate) fn recover_scope(
    ledger: &ChangedPathLedger<'_>,
    expected: &ExpectedScope,
) -> Result<Vec<(IntentId, RecoveryDecision)>> {
    let ids = pending_intent_ids(ledger.conn, &expected.scope_id.to_text())?;
    let mut decisions = Vec::with_capacity(ids.len());
    for id in ids {
        let tx = Transaction::new_unchecked(ledger.conn, TransactionBehavior::Immediate)?;
        let intent = load_intent(&tx, &id)?.ok_or_else(|| {
            Error::Corrupt(format!(
                "pending changed-path intent `{}` disappeared",
                id.0
            ))
        })?;
        if intent.state.is_terminal() {
            tx.commit()?;
            continue;
        }
        let exact_scope = intent_matches_expected_scope(&intent, expected)
            && scope_cas_matches(&tx, expected)?
            && current_change_matches(&tx, &intent)?;
        let target_published = authoritative_ref_matches_target(&tx, &intent)?;
        let qualified_proof = intent.verified_cut.as_ref().is_some_and(|proof| {
            ledger.database_path().is_ok_and(|database_path| {
                validate_qualified_filesystem_proof(&tx, database_path, expected, &intent, proof)
                    .is_ok()
            })
        });
        let decision = if exact_scope
            && target_published
            && matches!(
                intent.state,
                IntentState::FilesystemApplied | IntentState::Published
            )
            && qualified_proof
        {
            if intent.state == IntentState::FilesystemApplied {
                stage_intent_evidence(&tx, &intent)?;
                let published = tx.execute(
                    "UPDATE changed_path_intents SET lifecycle_state='published',updated_at=?1
                     WHERE intent_id=?2 AND lifecycle_state='filesystem_applied'",
                    params![now_ts(), intent.id.0],
                )?;
                if published != 1 {
                    return Err(Error::Corrupt(format!(
                        "intent `{}` changed during recovery publication",
                        intent.id.0
                    )));
                }
            }
            if finish_publication(&tx, &intent, expected)? {
                RecoveryDecision::FinishPublication
            } else {
                RecoveryDecision::FullReconciliation
            }
        } else if exact_scope && intent.state == IntentState::Prepared {
            retain_and_fail_closed(&tx, &intent, "prepared_intent_recovery_ambiguous")?;
            RecoveryDecision::FullReconciliation
        } else if exact_scope {
            retain_and_fail_closed(&tx, &intent, "intent_publication_state_ambiguous")?;
            RecoveryDecision::RetainCandidatesAndAcknowledge
        } else {
            retain_and_fail_closed(&tx, &intent, "intent_scope_identity_changed")?;
            RecoveryDecision::FullReconciliation
        };
        tx.commit()?;
        decisions.push((id, decision));
    }
    if !decisions.is_empty() {
        durable_intent_barrier(ledger.conn)?;
        crate::db::util::test_crash_point("changed_path_after_recovery_commit");
    }
    Ok(decisions)
}

impl ChangedPathLedger<'_> {
    pub(crate) fn recover_scope(
        &self,
        expected: &ExpectedScope,
    ) -> Result<Vec<(IntentId, RecoveryDecision)>> {
        recover_scope(self, expected)
    }

    pub(crate) fn recover(&self) -> Result<Vec<(IntentId, RecoveryDecision)>> {
        let scopes = scopes_with_pending_intents(self.conn)?;
        let mut decisions = Vec::new();
        for expected in scopes {
            decisions.extend(recover_scope(self, &expected)?);
        }
        Ok(decisions)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IntentGcRoot {
    pub(crate) change_id: crate::ChangeId,
    pub(crate) root_id: ObjectId,
    pub(crate) operation_id: Option<ObjectId>,
}

pub(crate) fn ledger_gc_roots(conn: &Connection) -> Result<Vec<IntentGcRoot>> {
    let mut roots = Vec::new();
    let mut statement = conn.prepare(
        "SELECT target_change_id,target_root_id,target_operation_id FROM changed_path_intents
         WHERE lifecycle_state IN ('prepared','filesystem_applied','published')",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, Option<String>>(2)?,
        ))
    })?;
    for row in rows {
        let (change, root, operation) = row?;
        roots.push(IntentGcRoot {
            change_id: crate::ChangeId(change),
            root_id: ObjectId(root),
            operation_id: operation.map(ObjectId),
        });
    }
    Ok(roots)
}

pub(crate) fn mark_backup_scopes_untrusted(conn: &Connection) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    tx.execute(
        "UPDATE changed_path_scopes
         SET trust_state='untrusted_gap',trust_reason='backup_without_observer_segments',
             continuity_generation=continuity_generation+1,updated_at=?1
         WHERE retired_at IS NULL",
        [now_ts()],
    )?;
    tx.execute(
        "UPDATE changed_path_intents SET lifecycle_state='aborted',
             failure_reason='backup_did_not_fence_observer_segments',updated_at=?1
         WHERE lifecycle_state IN ('prepared','filesystem_applied','published')",
        [now_ts()],
    )?;
    tx.execute("DELETE FROM changed_path_observer_owners", [])?;
    tx.execute("DELETE FROM changed_path_observer_segments", [])?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn rotate_restored_scopes(
    conn: &Connection,
    filesystem_identity: &[u8],
    scope_root: &str,
    previous_epoch: u64,
    previous_continuity_generation: u64,
) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let now = now_ts();
    let (source_epoch, source_continuity): (i64, i64) = tx.query_row(
        "SELECT COALESCE(MAX(epoch),0),COALESCE(MAX(continuity_generation),0)
         FROM changed_path_scopes WHERE retired_at IS NULL",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let next_epoch = db_u64(source_epoch, "restored source epoch")?
        .max(previous_epoch)
        .checked_add(1)
        .ok_or_else(|| Error::Corrupt("restored scope epoch overflow".into()))?;
    let next_continuity = db_u64(source_continuity, "restored source continuity")?
        .max(previous_continuity_generation)
        .checked_add(1)
        .ok_or_else(|| Error::Corrupt("restored scope continuity overflow".into()))?;
    tx.execute(
        "UPDATE changed_path_intents SET lifecycle_state='aborted',
             failure_reason='restore_rotated_scope_identity',updated_at=?1
         WHERE lifecycle_state IN ('prepared','filesystem_applied','published')",
        [now],
    )?;
    tx.execute("DELETE FROM changed_path_observer_owners", [])?;
    tx.execute("DELETE FROM changed_path_observer_segments", [])?;
    tx.execute(
        "UPDATE changed_path_scopes SET epoch=?1,scope_root=?2,
             filesystem_identity=?3,scope_root_identity=?3,
             provider_id=NULL,provider_identity=NULL,provider_cursor=NULL,provider_fence=NULL,
             observer_owner_token=NULL,observer_heartbeat_at=NULL,
             durable_offset=0,folded_offset=0,trust_state='untrusted_gap',
             trust_reason='restored_filesystem_identity_rotated',
             continuity_generation=?4,updated_at=?5
         WHERE retired_at IS NULL",
        params![
            sql_u64(next_epoch, "restored target epoch")?,
            scope_root,
            hex::encode(filesystem_identity),
            sql_u64(next_continuity, "restored target continuity")?,
            now
        ],
    )?;
    tx.commit()?;
    Ok(())
}

#[derive(Clone)]
pub(crate) struct SegmentDeletionToken {
    scope_id: super::ScopeId,
    epoch: u64,
    segment_id: String,
    directory: std::sync::Arc<super::secure_fs::SecureDirectory>,
    identity: PersistedSegmentDeletion,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PersistedSegmentDeletion {
    original_leaf: String,
    quarantine_leaf: String,
    scope_directory_identity: (u64, u64),
    quarantine_identity: (u64, u64),
    segment_identity: (u64, u64),
    file_length: u64,
    file_hash: [u8; 32],
    durable_end_offset: u64,
    durable_hash: [u8; 32],
    limits: super::PersistedLogLimits,
    owner_token: [u8; 32],
    first_sequence: u64,
    last_sequence: Option<u64>,
    previous_segment_id: Option<String>,
    previous_segment_hash: [u8; 32],
    source_state: String,
    state: String,
}

impl std::fmt::Debug for SegmentDeletionToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SegmentDeletionToken")
            .field("scope_id", &self.scope_id.to_text())
            .field("epoch", &self.epoch)
            .field("segment_id", &self.segment_id)
            .field("leaf", &self.identity.original_leaf)
            .field("quarantine_leaf", &self.identity.quarantine_leaf)
            .field("state", &self.identity.state)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RetirementIdentity {
    scope_id: super::ScopeId,
    epoch: u64,
    scope_kind: String,
    owner_id: String,
    scope_root: String,
    ref_name: String,
    ref_generation: u64,
    change_id: String,
    baseline_root_id: ObjectId,
    policy_fingerprint: String,
    policy_generation: u64,
    filesystem_identity: String,
    continuity_generation: u64,
    provider_identity: Option<String>,
    retired_at: Option<i64>,
}

#[cfg(debug_assertions)]
#[derive(Clone, Copy, Eq, PartialEq)]
enum DeletionSubstitutionPoint {
    Parent,
    BeforeQuarantineMove,
    AfterDirectRenameBeforeVerify,
}

#[cfg(debug_assertions)]
thread_local! {
    static DELETION_SUBSTITUTION_HOOK: std::cell::RefCell<
        Option<(DeletionSubstitutionPoint, Box<dyn FnOnce()>)>
    > = const { std::cell::RefCell::new(None) };
}

#[cfg(debug_assertions)]
fn install_deletion_substitution_hook(
    point: DeletionSubstitutionPoint,
    hook: impl FnOnce() + 'static,
) {
    DELETION_SUBSTITUTION_HOOK.with(|slot| {
        *slot.borrow_mut() = Some((point, Box::new(hook)));
    });
}

#[cfg(debug_assertions)]
fn run_deletion_substitution_hook(point: DeletionSubstitutionPoint) {
    DELETION_SUBSTITUTION_HOOK.with(|slot| {
        let should_run = slot
            .borrow()
            .as_ref()
            .is_some_and(|(installed, _)| *installed == point);
        if should_run {
            if let Some((_, hook)) = slot.borrow_mut().take() {
                hook();
            }
        }
    });
}

#[cfg(debug_assertions)]
fn clear_deletion_substitution_hook() {
    DELETION_SUBSTITUTION_HOOK.with(|slot| {
        slot.borrow_mut().take();
    });
}

pub(crate) fn retire_scope(
    conn: &Connection,
    database_path: &std::path::Path,
    expected: &ExpectedScope,
) -> Result<Vec<SegmentDeletionToken>> {
    let identity =
        load_retirement_identity(conn, &expected.scope_id.to_text())?.ok_or_else(|| {
            Error::ChangeLedgerReconcileRequired {
                scope: expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: "scope retirement lookup failed".into(),
                command: "trail status".into(),
            }
        })?;
    let expected_provider_identity = hex::encode(&expected.provider_identity);
    if identity.epoch != expected.epoch
        || identity.ref_name != expected.ref_name
        || identity.ref_generation != expected.ref_generation
        || identity.baseline_root_id != expected.baseline_root
        || identity.policy_fingerprint != hex::encode(expected.policy_fingerprint)
        || identity.policy_generation != expected.policy_generation
        || identity.filesystem_identity != hex::encode(&expected.filesystem_identity)
        || identity.provider_identity.as_deref() != Some(expected_provider_identity.as_str())
    {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "scope retirement expected identity changed".into(),
            command: "trail status".into(),
        });
    }
    retire_scope_identity(conn, database_path, &identity)
}

fn retire_scope_identity(
    conn: &Connection,
    database_path: &std::path::Path,
    expected: &RetirementIdentity,
) -> Result<Vec<SegmentDeletionToken>> {
    let retiring = if expected.retired_at.is_none() {
        begin_scope_retirement(conn, expected)?
    } else {
        expected.clone()
    };
    let quiesced_rows = if retiring.retired_at.is_none() {
        ensure_segment_quarantine_allocations(conn, database_path, &retiring)?
    } else {
        Vec::new()
    };
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let now = now_ts();
    let current_matches: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM changed_path_scopes
         WHERE scope_id=?1 AND epoch=?2 AND scope_kind=?3 AND owner_id=?4 AND scope_root=?5
           AND ref_name=?6 AND ref_generation=?7 AND change_id=?8 AND baseline_root_id=?9
           AND policy_fingerprint=?10 AND policy_dependency_generation=?11
           AND filesystem_identity=?12 AND continuity_generation=?13
           AND provider_identity IS ?14 AND retired_at IS ?15)",
        params![
            retiring.scope_id.to_text(),
            sql_u64(retiring.epoch, "scope epoch")?,
            retiring.scope_kind,
            retiring.owner_id,
            retiring.scope_root,
            retiring.ref_name,
            sql_u64(retiring.ref_generation, "ref generation")?,
            retiring.change_id,
            retiring.baseline_root_id.0,
            retiring.policy_fingerprint,
            sql_u64(retiring.policy_generation, "policy generation")?,
            retiring.filesystem_identity,
            sql_u64(retiring.continuity_generation, "continuity generation")?,
            retiring.provider_identity,
            retiring.retired_at,
        ],
        |row| row.get(0),
    )?;
    if !current_matches {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: retiring.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "scope retirement exact row changed".into(),
            command: "trail status".into(),
        });
    };
    if retiring.retired_at.is_none() {
        prepare_segment_deletion_transactions(
            &tx,
            database_path,
            retiring.scope_id,
            retiring.epoch,
            &quiesced_rows,
        )?;
    } else {
        validate_segment_deletion_transaction_coverage(&tx, retiring.scope_id, retiring.epoch)?;
    }
    if retiring.retired_at.is_some() {
        tx.commit()?;
        durable_intent_barrier(conn)?;
        return load_segment_deletion_tokens(
            conn,
            database_path,
            retiring.scope_id,
            retiring.epoch,
        );
    }
    let changed = tx.execute(
        "UPDATE changed_path_scopes SET retired_at=?1,trust_state='untrusted_gap',
             trust_reason='scope_retired',
             observer_owner_token=NULL,observer_heartbeat_at=NULL,updated_at=?1
         WHERE scope_id=?2 AND epoch=?3 AND continuity_generation=?4
           AND provider_identity IS ?5 AND retired_at IS NULL
           AND trust_state='untrusted_gap' AND trust_reason='scope_retiring'",
        params![
            now,
            retiring.scope_id.to_text(),
            sql_u64(retiring.epoch, "scope epoch")?,
            sql_u64(retiring.continuity_generation, "continuity generation")?,
            retiring.provider_identity,
        ],
    )?;
    if changed != 1 {
        let observed: (i64, i64, Option<String>, Option<i64>) = tx.query_row(
            "SELECT epoch,continuity_generation,provider_identity,retired_at
             FROM changed_path_scopes WHERE scope_id=?1",
            [retiring.scope_id.to_text()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: retiring.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: format!(
                "scope retirement update CAS failed: expected epoch={} continuity={} provider={:?}; observed={observed:?}",
                retiring.epoch, retiring.continuity_generation, retiring.provider_identity
            ),
            command: "trail status".into(),
        });
    }
    tx.execute(
        "UPDATE changed_path_observer_segments SET state='retired',updated_at=?1
         WHERE scope_id=?2 AND epoch=?3 AND state='retiring'",
        params![
            now,
            retiring.scope_id.to_text(),
            sql_u64(retiring.epoch, "scope epoch")?
        ],
    )?;
    crate::db::util::test_crash_point("changed_path_deletion_before_retirement_commit");
    tx.commit()?;
    crate::db::util::test_crash_point("changed_path_deletion_after_retirement_commit");
    durable_intent_barrier(conn)?;
    crate::db::util::test_crash_point("changed_path_deletion_after_retirement_wal_barrier");
    load_segment_deletion_tokens(conn, database_path, retiring.scope_id, retiring.epoch)
}

fn begin_scope_retirement(
    conn: &Connection,
    expected: &RetirementIdentity,
) -> Result<RetirementIdentity> {
    let trust_reason: String = conn.query_row(
        "SELECT trust_reason FROM changed_path_scopes WHERE scope_id=?1",
        [expected.scope_id.to_text()],
        |row| row.get(0),
    )?;
    if trust_reason == "scope_retiring" {
        return load_retirement_identity(conn, &expected.scope_id.to_text())?
            .ok_or_else(|| Error::Corrupt("retiring scope disappeared".into()));
    }
    let (_, rows) = load_retired_segment_rows(conn, expected.scope_id, expected.epoch)?;
    if rows
        .iter()
        .any(|row| !matches!(row.state.as_str(), "open" | "sealed"))
    {
        return Err(Error::Corrupt(
            "scope retirement started from an invalid observer segment state".into(),
        ));
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let now = now_ts();
    let current_matches: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM changed_path_scopes
         WHERE scope_id=?1 AND epoch=?2 AND continuity_generation=?3
           AND provider_identity IS ?4 AND retired_at IS NULL)",
        params![
            expected.scope_id.to_text(),
            sql_u64(expected.epoch, "scope epoch")?,
            sql_u64(expected.continuity_generation, "continuity generation")?,
            expected.provider_identity,
        ],
        |row| row.get(0),
    )?;
    if !current_matches {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "scope changed before retirement revocation fence".into(),
            command: "trail status".into(),
        });
    }
    let mut fence_nonce = [0_u8; 32];
    getrandom::getrandom(&mut fence_nonce).map_err(|error| {
        Error::InvalidInput(format!("retirement fence nonce generation failed: {error}"))
    })?;
    let revoked = tx.execute(
        "UPDATE changed_path_observer_owners
         SET lease_state='revoked',fence_nonce=?1,updated_at=?2
         WHERE scope_id=?3 AND epoch=?4 AND lease_state='active'",
        params![
            fence_nonce,
            now,
            expected.scope_id.to_text(),
            sql_u64(expected.epoch, "scope epoch")?,
        ],
    )?;
    let post_trigger_continuity = expected
        .continuity_generation
        .checked_add(u64::from(revoked != 0))
        .ok_or_else(|| Error::Corrupt("retirement continuity generation overflow".into()))?;
    let changed = tx.execute(
        "UPDATE changed_path_scopes
         SET trust_state='untrusted_gap',trust_reason='scope_retiring',
             continuity_generation=continuity_generation+?1,
             observer_owner_token=NULL,observer_heartbeat_at=NULL,updated_at=?2
         WHERE scope_id=?3 AND epoch=?4 AND continuity_generation=?5
           AND provider_identity IS ?6 AND retired_at IS NULL",
        params![
            i64::from(revoked == 0),
            now,
            expected.scope_id.to_text(),
            sql_u64(expected.epoch, "scope epoch")?,
            sql_u64(post_trigger_continuity, "continuity generation")?,
            expected.provider_identity,
        ],
    )?;
    if changed != 1 {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "scope retirement revocation fence CAS failed".into(),
            command: "trail status".into(),
        });
    }
    tx.execute(
        "UPDATE changed_path_observer_segments
         SET retirement_source_state=state,state='retiring',updated_at=?1
         WHERE scope_id=?2 AND epoch=?3 AND state IN ('open','sealed')",
        params![
            now,
            expected.scope_id.to_text(),
            sql_u64(expected.epoch, "scope epoch")?,
        ],
    )?;
    tx.commit()?;
    durable_intent_barrier(conn)?;
    crate::db::util::test_crash_point("changed_path_deletion_after_retirement_fence_barrier");
    load_retirement_identity(conn, &expected.scope_id.to_text())?
        .ok_or_else(|| Error::Corrupt("retiring scope disappeared after fence".into()))
}

fn load_retirement_identity(
    conn: &Connection,
    scope_id: &str,
) -> Result<Option<RetirementIdentity>> {
    let row = conn
        .query_row(
            "SELECT scope_id,epoch,scope_kind,owner_id,scope_root,ref_name,ref_generation,
                    change_id,baseline_root_id,policy_fingerprint,policy_dependency_generation,
                    filesystem_identity,continuity_generation,provider_identity,retired_at
             FROM changed_path_scopes WHERE scope_id=?1",
            [scope_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, i64>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, Option<i64>>(14)?,
                ))
            },
        )
        .optional()?;
    let Some(row) = row else {
        return Ok(None);
    };
    let scope_bytes = hex::decode(&row.0).map_err(|error| Error::Corrupt(error.to_string()))?;
    Ok(Some(RetirementIdentity {
        scope_id: super::ScopeId(
            scope_bytes
                .try_into()
                .map_err(|_| Error::Corrupt("invalid retirement scope id".into()))?,
        ),
        epoch: db_u64(row.1, "retirement scope epoch")?,
        scope_kind: row.2,
        owner_id: row.3,
        scope_root: row.4,
        ref_name: row.5,
        ref_generation: db_u64(row.6, "retirement ref generation")?,
        change_id: row.7,
        baseline_root_id: ObjectId(row.8),
        policy_fingerprint: row.9,
        policy_generation: db_u64(row.10, "retirement policy generation")?,
        filesystem_identity: row.11,
        continuity_generation: db_u64(row.12, "retirement continuity generation")?,
        provider_identity: row.13,
        retired_at: row.14,
    }))
}

pub(crate) fn retire_deletion_scopes(
    conn: &Connection,
    database_path: &std::path::Path,
    owner_ids: &[&str],
    scope_roots: &[&str],
    ref_names: &[&str],
) -> Result<Vec<SegmentDeletionToken>> {
    let scope_ids = {
        let mut statement = conn.prepare(
            "SELECT scope_id,owner_id,scope_root,ref_name
             FROM changed_path_scopes
             WHERE scope_kind IN ('materialized_lane','workspace_view')
             ORDER BY scope_id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .filter_map(|(scope_id, owner_id, scope_root, ref_name)| {
                (owner_ids.contains(&owner_id.as_str())
                    || scope_roots.contains(&scope_root.as_str())
                    || ref_names.contains(&ref_name.as_str()))
                .then_some(scope_id)
            })
            .collect::<Vec<_>>();
        rows
    };
    let mut retired_paths = Vec::new();
    for scope_id in scope_ids {
        let identity = load_retirement_identity(conn, &scope_id)?.ok_or_else(|| {
            Error::ChangeLedgerReconcileRequired {
                scope: scope_id.clone(),
                state: "untrusted_gap".into(),
                reason: "deletion scope disappeared before retirement".into(),
                command: "trail status".into(),
            }
        })?;
        retired_paths.extend(retire_scope_identity(conn, database_path, &identity)?);
    }
    Ok(retired_paths)
}

fn validate_retired_segment_paths(paths: &[String]) -> Result<()> {
    let mut unique = std::collections::HashSet::with_capacity(paths.len());
    for path in paths {
        let parsed = std::path::Path::new(path);
        let mut components = parsed.components();
        let confined = matches!(
            (components.next(), components.next()),
            (Some(std::path::Component::Normal(_)), None)
        );
        if !confined
            || path
                .chars()
                .any(|character| matches!(character, '/' | '\\' | '\0'))
            || !path.ends_with(".cpl")
            || path.len() > 128
            || !unique.insert(path)
        {
            return Err(Error::Corrupt(format!(
                "observer segment retirement path is not confined: `{path}`"
            )));
        }
    }
    Ok(())
}

#[derive(Debug)]
struct RetiredSegmentRow {
    segment_id: String,
    log_format_version: u64,
    owner_token: [u8; 32],
    provider_id: String,
    first_sequence: u64,
    last_sequence: Option<u64>,
    durable_end_offset: u64,
    folded_end_offset: u64,
    previous_segment_id: Option<String>,
    previous_segment_hash: [u8; 32],
    stored_segment_hash: Option<[u8; 32]>,
    segment_path: String,
    state: String,
    source_state: String,
    file_length: Option<u64>,
    file_hash: Option<[u8; 32]>,
    durable_hash: Option<[u8; 32]>,
    source_identity: Option<(u64, u64)>,
    quiescence_file: Option<std::fs::File>,
}

#[derive(Clone, Debug)]
struct QuarantineAllocation {
    attempt_nonce: String,
    quarantine_leaf: String,
    scope_directory_identity: (u64, u64),
    source_identity: (u64, u64),
    quarantine_identity: Option<(u64, u64)>,
    state: String,
}

fn allocation_quarantine_leaf(
    scope_id: super::ScopeId,
    epoch: u64,
    segment_id: &str,
    attempt_nonce: &str,
) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    hasher.update(b"trail-segment-direct-quarantine-v1\0");
    hasher.update(scope_id.0);
    hasher.update(epoch.to_le_bytes());
    hasher.update(segment_id.as_bytes());
    hasher.update(attempt_nonce.as_bytes());
    format!(".trail-delete-{}.cplq", hex::encode(hasher.finalize()))
}

fn new_allocation_nonce() -> Result<String> {
    let mut nonce = [0_u8; 32];
    getrandom::getrandom(&mut nonce).map_err(|error| {
        Error::InvalidInput(format!("allocation nonce generation failed: {error}"))
    })?;
    Ok(hex::encode(nonce))
}

fn insert_quarantine_allocation(
    tx: &Transaction<'_>,
    scope_id: super::ScopeId,
    epoch: u64,
    segment_id: &str,
    scope_directory_identity: (u64, u64),
    source_identity: (u64, u64),
) -> Result<()> {
    let attempt_nonce = new_allocation_nonce()?;
    let quarantine_leaf = allocation_quarantine_leaf(scope_id, epoch, segment_id, &attempt_nonce);
    let now = now_ts();
    tx.execute(
        "INSERT INTO changed_path_segment_quarantine_allocations(
             attempt_nonce,scope_id,epoch,segment_id,quarantine_leaf,
             scope_directory_device,scope_directory_inode,identity_policy,
             source_segment_device,source_segment_inode,quarantine_device,quarantine_inode,
             observed_conflict_device,observed_conflict_inode,retained_reason,state,
             created_at,updated_at,allocated_at,bound_at,abandoned_at)
         VALUES(?1,?2,?3,?4,?5,?6,?7,'direct_noreplace_same_directory_v1',
                ?8,?9,NULL,NULL,NULL,NULL,NULL,'allocating',?10,?10,NULL,NULL,NULL)",
        params![
            attempt_nonce,
            scope_id.to_text(),
            sql_u64(epoch, "scope epoch")?,
            segment_id,
            quarantine_leaf,
            encode_fs_identity_part(scope_directory_identity.0),
            encode_fs_identity_part(scope_directory_identity.1),
            encode_fs_identity_part(source_identity.0),
            encode_fs_identity_part(source_identity.1),
            now,
        ],
    )?;
    Ok(())
}

fn load_active_quarantine_allocation(
    conn: &Connection,
    scope_id: super::ScopeId,
    epoch: u64,
    segment_id: &str,
) -> Result<QuarantineAllocation> {
    let rows = conn
        .prepare(
            "SELECT attempt_nonce,quarantine_leaf,
                    scope_directory_device,scope_directory_inode,
                    source_segment_device,source_segment_inode,
                    quarantine_device,quarantine_inode,state,identity_policy
             FROM changed_path_segment_quarantine_allocations
             WHERE scope_id=?1 AND epoch=?2 AND segment_id=?3
               AND state IN ('allocating','allocated','bound')",
        )?
        .query_map(
            params![
                scope_id.to_text(),
                sql_u64(epoch, "scope epoch")?,
                segment_id
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                ))
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if rows.len() != 1 {
        return Err(Error::Corrupt(format!(
            "segment requires exactly one active quarantine allocation: {}/{epoch}/{segment_id} observed={}",
            scope_id.to_text(),
            rows.len()
        )));
    }
    let row = &rows[0];
    if row.9 != "direct_noreplace_same_directory_v1" || row.6.is_some() != row.7.is_some() {
        return Err(Error::Corrupt(
            "quarantine allocation identity policy or identity pair is invalid".into(),
        ));
    }
    Ok(QuarantineAllocation {
        attempt_nonce: row.0.clone(),
        quarantine_leaf: row.1.clone(),
        scope_directory_identity: (
            decode_fs_identity_part(&row.2, "allocation scope directory device")?,
            decode_fs_identity_part(&row.3, "allocation scope directory inode")?,
        ),
        source_identity: (
            decode_fs_identity_part(&row.4, "allocation source device")?,
            decode_fs_identity_part(&row.5, "allocation source inode")?,
        ),
        quarantine_identity: row
            .6
            .as_deref()
            .zip(row.7.as_deref())
            .map(|(device, inode)| {
                Ok::<(u64, u64), Error>((
                    decode_fs_identity_part(device, "allocation quarantine device")?,
                    decode_fs_identity_part(inode, "allocation quarantine inode")?,
                ))
            })
            .transpose()?,
        state: row.8.clone(),
    })
}

fn load_retired_segment_rows(
    conn: &Connection,
    scope_id: super::ScopeId,
    epoch: u64,
) -> Result<(super::PersistedLogLimits, Vec<RetiredSegmentRow>)> {
    let limit_values: (i64, i64, i64) = conn.query_row(
        "SELECT max_observer_log_bytes,max_segment_bytes,max_unfolded_tail_records
         FROM changed_path_scopes WHERE scope_id=?1 AND epoch=?2",
        params![scope_id.to_text(), sql_u64(epoch, "scope epoch")?],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let limits = super::PersistedLogLimits {
        max_log_bytes: db_u64(limit_values.0, "maximum observer log bytes")?,
        max_segment_bytes: db_u64(limit_values.1, "maximum segment bytes")?,
        max_unfolded_tail_records: usize::try_from(db_u64(
            limit_values.2,
            "maximum unfolded records",
        )?)
        .map_err(|_| Error::Corrupt("maximum unfolded records exceeds memory".into()))?,
    };
    let rows = conn
        .prepare(
            "SELECT segment_id,owner_token,first_sequence,last_sequence,durable_end_offset,
                    previous_segment_id,previous_segment_hash,segment_hash,segment_path,state,
                    COALESCE(retirement_source_state,state),log_format_version,provider_id,
                    folded_end_offset,retirement_file_length,retirement_file_hash,
                    retirement_durable_hash,retirement_source_device,retirement_source_inode
             FROM changed_path_observer_segments
             WHERE scope_id=?1 AND epoch=?2 ORDER BY first_sequence,segment_id",
        )?
        .query_map(
            params![scope_id.to_text(), sql_u64(epoch, "scope epoch")?],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, Option<i64>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, Option<String>>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, i64>(11)?,
                    row.get::<_, String>(12)?,
                    row.get::<_, i64>(13)?,
                    row.get::<_, Option<i64>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, Option<String>>(16)?,
                    row.get::<_, Option<String>>(17)?,
                    row.get::<_, Option<String>>(18)?,
                ))
            },
        )?
        .collect::<std::result::Result<Vec<_>, _>>()?
        .into_iter()
        .map(|row| {
            Ok(RetiredSegmentRow {
                segment_id: row.0,
                log_format_version: db_u64(row.11, "retired segment log format")?,
                owner_token: decode_hex_32(&row.1, "retired segment owner token")?,
                provider_id: row.12,
                first_sequence: db_u64(row.2, "retired segment first sequence")?,
                last_sequence: row
                    .3
                    .map(|value| db_u64(value, "retired segment last sequence"))
                    .transpose()?,
                durable_end_offset: db_u64(row.4, "retired segment durable offset")?,
                folded_end_offset: db_u64(row.13, "retired segment folded offset")?,
                previous_segment_id: row.5,
                previous_segment_hash: row
                    .6
                    .as_deref()
                    .map(|value| decode_hex_32(value, "retired previous segment hash"))
                    .transpose()?
                    .unwrap_or([0; 32]),
                stored_segment_hash: row
                    .7
                    .as_deref()
                    .map(|value| decode_hex_32(value, "retired segment hash"))
                    .transpose()?,
                segment_path: row.8,
                state: row.9,
                source_state: row.10,
                file_length: row
                    .14
                    .map(|value| db_u64(value, "retirement file length"))
                    .transpose()?,
                file_hash: row
                    .15
                    .as_deref()
                    .map(|value| decode_hex_32(value, "retirement file hash"))
                    .transpose()?,
                durable_hash: row
                    .16
                    .as_deref()
                    .map(|value| decode_hex_32(value, "retirement durable hash"))
                    .transpose()?,
                source_identity: row
                    .17
                    .as_deref()
                    .zip(row.18.as_deref())
                    .map(|(device, inode)| {
                        Ok::<_, Error>((
                            decode_fs_identity_part(device, "retirement source device")?,
                            decode_fs_identity_part(inode, "retirement source inode")?,
                        ))
                    })
                    .transpose()?,
                quiescence_file: None,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    validate_retired_segment_paths(
        &rows
            .iter()
            .map(|row| row.segment_path.clone())
            .collect::<Vec<_>>(),
    )?;
    Ok((limits, rows))
}

fn scope_segment_directory_path(
    database_path: &std::path::Path,
    scope_id: super::ScopeId,
) -> Result<std::path::PathBuf> {
    let trail_root = database_path
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or_else(|| Error::Corrupt("retirement database has no Trail root".into()))?;
    Ok(trail_root
        .join("observer-segments")
        .join(scope_id.to_text()))
}

fn inspect_segments_before_allocation(
    conn: &Connection,
    database_path: &std::path::Path,
    scope_id: super::ScopeId,
    epoch: u64,
) -> Result<((u64, u64), Vec<RetiredSegmentRow>)> {
    let (limits, mut rows) = load_retired_segment_rows(conn, scope_id, epoch)?;
    if rows.is_empty() {
        return Ok(((0, 0), rows));
    }
    let directory = super::secure_fs::SecureDirectory::open_absolute(
        &scope_segment_directory_path(database_path, scope_id)?,
    )?;
    let scope_directory_identity = directory.identity()?;
    let mut expected_previous_segment_id: Option<String> = None;
    let mut expected_previous_segment_hash = [0; 32];
    let mut expected_first_sequence = 1_u64;
    let mut saw_open_segment = false;
    for row in &mut rows {
        if saw_open_segment
            || row.previous_segment_id != expected_previous_segment_id
            || row.previous_segment_hash != expected_previous_segment_hash
            || row.first_sequence != expected_first_sequence
        {
            return Err(Error::Corrupt(format!(
                "retired observer segment metadata lineage is not exact at `{}`",
                row.segment_id
            )));
        }
        let file = match directory.open_regular(&row.segment_path) {
            Ok(file) => file,
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                let allocation =
                    load_active_quarantine_allocation(conn, scope_id, epoch, &row.segment_id)?;
                let file = directory.open_regular(&allocation.quarantine_leaf)?;
                let identity = super::secure_fs::file_identity(&file)?;
                if identity != allocation.source_identity {
                    return Err(Error::InvalidInput(
                        "journaled quarantine does not match the authenticated source inode".into(),
                    ));
                }
                file
            }
            Err(error) => return Err(error),
        };
        let source_identity = super::secure_fs::file_identity(&file)?;
        match super::secure_fs::try_lock_observer_quiescence(&file) {
            Ok(()) => {}
            Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock => {
                return Err(Error::WorkspaceLocked(format!(
                    "observer segment writer has not acknowledged close: {}",
                    row.segment_id
                )));
            }
            Err(error) => return Err(error),
        }
        let authenticated = super::log::authenticate_segment_for_deletion(
            &file,
            &super::log::DeletionSegmentExpectation {
                scope_id,
                epoch,
                segment_id: row.segment_id.clone(),
                owner_token: row.owner_token,
                first_sequence: row.first_sequence,
                last_sequence: row.last_sequence,
                durable_end_offset: row.durable_end_offset,
                previous_segment_hash: row.previous_segment_hash,
                stored_segment_hash: row.stored_segment_hash,
                state: row.source_state.clone(),
                limits,
            },
        )
        .map_err(|error| Error::Corrupt(error.to_string()))?;
        if row
            .file_length
            .is_some_and(|value| value != authenticated.file_length)
            || row
                .file_hash
                .is_some_and(|value| value != authenticated.file_hash)
            || row
                .durable_hash
                .is_some_and(|value| value != authenticated.durable_hash)
            || row
                .source_identity
                .is_some_and(|value| value != source_identity)
        {
            return Err(Error::Corrupt(
                "persisted retirement source metadata changed before allocation".into(),
            ));
        }
        row.file_length = Some(authenticated.file_length);
        row.file_hash = Some(authenticated.file_hash);
        row.durable_hash = Some(authenticated.durable_hash);
        row.source_identity = Some(source_identity);
        row.quiescence_file = Some(file);
        expected_previous_segment_id = Some(row.segment_id.clone());
        expected_previous_segment_hash = authenticated.durable_hash;
        saw_open_segment = row.source_state == "open";
        expected_first_sequence = row
            .last_sequence
            .unwrap_or(row.first_sequence.saturating_sub(1))
            .checked_add(1)
            .ok_or_else(|| Error::Corrupt("retired segment sequence overflow".into()))?;
    }
    Ok((scope_directory_identity, rows))
}

fn journal_missing_quarantine_allocations(
    conn: &Connection,
    expected: &RetirementIdentity,
    scope_directory_identity: (u64, u64),
    rows: &[RetiredSegmentRow],
) -> Result<()> {
    if rows.is_empty() {
        return Ok(());
    }
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if load_retirement_identity(&tx, &expected.scope_id.to_text())?.as_ref() != Some(expected) {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "scope identity changed before quarantine allocation journal".into(),
            command: "trail status".into(),
        });
    }
    for row in rows {
        let source_identity = row.source_identity.ok_or_else(|| {
            Error::Corrupt("retirement source identity was not authenticated".into())
        })?;
        let file_length = row
            .file_length
            .ok_or_else(|| Error::Corrupt("retirement file length was not authenticated".into()))?;
        let file_hash = row
            .file_hash
            .ok_or_else(|| Error::Corrupt("retirement file hash was not authenticated".into()))?;
        let durable_hash = row.durable_hash.ok_or_else(|| {
            Error::Corrupt("retirement durable hash was not authenticated".into())
        })?;
        let source_published = tx.execute(
            "UPDATE changed_path_observer_segments
             SET retirement_file_length=?1,retirement_file_hash=?2,
                 retirement_durable_hash=?3,retirement_source_device=?4,
                 retirement_source_inode=?5,updated_at=?6
             WHERE scope_id=?7 AND epoch=?8 AND segment_id=?9 AND state='retiring'
               AND owner_token=?10 AND provider_id=?11 AND log_format_version=?12
               AND first_sequence=?13 AND last_sequence IS ?14
               AND durable_end_offset=?15 AND folded_end_offset=?16
               AND segment_path=?17 AND retirement_source_state=?18",
            params![
                sql_u64(file_length, "retirement file length")?,
                hex::encode(file_hash),
                hex::encode(durable_hash),
                encode_fs_identity_part(source_identity.0),
                encode_fs_identity_part(source_identity.1),
                now_ts(),
                expected.scope_id.to_text(),
                sql_u64(expected.epoch, "scope epoch")?,
                row.segment_id,
                hex::encode(row.owner_token),
                row.provider_id,
                sql_u64(row.log_format_version, "segment log format")?,
                sql_u64(row.first_sequence, "segment first sequence")?,
                row.last_sequence
                    .map(|value| sql_u64(value, "segment last sequence"))
                    .transpose()?,
                sql_u64(row.durable_end_offset, "segment durable offset")?,
                sql_u64(row.folded_end_offset, "segment folded offset")?,
                row.segment_path,
                row.source_state,
            ],
        )?;
        if source_published != 1 {
            return Err(Error::Conflict(format!(
                "retirement source metadata changed before publication: {}",
                row.segment_id
            )));
        }
        let active: i64 = tx.query_row(
            "SELECT COUNT(*) FROM changed_path_segment_quarantine_allocations
             WHERE scope_id=?1 AND epoch=?2 AND segment_id=?3
               AND state IN ('allocating','allocated','bound')",
            params![
                expected.scope_id.to_text(),
                sql_u64(expected.epoch, "scope epoch")?,
                row.segment_id
            ],
            |sql_row| sql_row.get(0),
        )?;
        match active {
            0 => insert_quarantine_allocation(
                &tx,
                expected.scope_id,
                expected.epoch,
                &row.segment_id,
                scope_directory_identity,
                source_identity,
            )?,
            1 => {}
            count => {
                return Err(Error::Corrupt(format!(
                    "multiple active quarantine allocations exist for segment `{}`: {count}",
                    row.segment_id
                )));
            }
        }
    }
    tx.commit()?;
    durable_intent_barrier(conn)?;
    crate::db::util::test_crash_point("changed_path_deletion_after_allocation_journal_barrier");
    Ok(())
}

fn abandon_quarantine_allocation_and_replace(
    conn: &Connection,
    expected: &RetirementIdentity,
    segment_id: &str,
    allocation: &QuarantineAllocation,
    observed_identity: Option<(u64, u64)>,
    reason: &str,
    replace: bool,
) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    if load_retirement_identity(&tx, &expected.scope_id.to_text())?.as_ref() != Some(expected) {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "scope identity changed while replacing quarantine allocation".into(),
            command: "trail status".into(),
        });
    }
    let now = now_ts();
    let changed = tx.execute(
        "UPDATE changed_path_segment_quarantine_allocations
         SET state='abandoned',observed_conflict_device=?1,observed_conflict_inode=?2,
             retained_reason=?3,updated_at=?4,abandoned_at=?4
         WHERE attempt_nonce=?5 AND state=?6",
        params![
            observed_identity.map(|identity| encode_fs_identity_part(identity.0)),
            observed_identity.map(|identity| encode_fs_identity_part(identity.1)),
            reason,
            now,
            allocation.attempt_nonce,
            allocation.state,
        ],
    )?;
    if changed != 1 {
        return Err(Error::Conflict(
            "quarantine allocation changed while abandoning it".into(),
        ));
    }
    if replace {
        insert_quarantine_allocation(
            &tx,
            expected.scope_id,
            expected.epoch,
            segment_id,
            allocation.scope_directory_identity,
            allocation.source_identity,
        )?;
    }
    tx.commit()?;
    durable_intent_barrier(conn)
}

fn mark_quarantine_allocation_allocated(
    conn: &Connection,
    allocation: &QuarantineAllocation,
    identity: (u64, u64),
) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let now = now_ts();
    let changed = tx.execute(
        "UPDATE changed_path_segment_quarantine_allocations
         SET state='allocated',quarantine_device=?1,
             quarantine_inode=?2,updated_at=?3,allocated_at=?3
         WHERE attempt_nonce=?4 AND state='allocating'
           AND quarantine_device IS NULL AND quarantine_inode IS NULL",
        params![
            encode_fs_identity_part(identity.0),
            encode_fs_identity_part(identity.1),
            now,
            allocation.attempt_nonce,
        ],
    )?;
    if changed != 1 {
        return Err(Error::Conflict(
            "quarantine allocation changed before identity publication".into(),
        ));
    }
    tx.commit()?;
    durable_intent_barrier(conn)?;
    crate::db::util::test_crash_point("changed_path_deletion_after_allocation_identity_barrier");
    Ok(())
}

fn authenticate_direct_quarantine(
    file: &std::fs::File,
    row: &RetiredSegmentRow,
    scope_id: super::ScopeId,
    epoch: u64,
    limits: super::PersistedLogLimits,
) -> Result<()> {
    super::log::authenticate_segment_for_deletion(
        file,
        &super::log::DeletionSegmentExpectation {
            scope_id,
            epoch,
            segment_id: row.segment_id.clone(),
            owner_token: row.owner_token,
            first_sequence: row.first_sequence,
            last_sequence: row.last_sequence,
            durable_end_offset: row.durable_end_offset,
            previous_segment_hash: row.previous_segment_hash,
            stored_segment_hash: row.stored_segment_hash,
            state: row.source_state.clone(),
            limits,
        },
    )
    .map(|_| ())
    .map_err(|error| Error::Corrupt(error.to_string()))
}

fn ensure_segment_quarantine_allocations(
    conn: &Connection,
    database_path: &std::path::Path,
    expected: &RetirementIdentity,
) -> Result<Vec<RetiredSegmentRow>> {
    let (scope_directory_identity, rows) =
        inspect_segments_before_allocation(conn, database_path, expected.scope_id, expected.epoch)?;
    if rows.is_empty() {
        return Ok(rows);
    }
    journal_missing_quarantine_allocations(conn, expected, scope_directory_identity, &rows)?;
    let directory = super::secure_fs::SecureDirectory::open_absolute(
        &scope_segment_directory_path(database_path, expected.scope_id)?,
    )?;
    let (limits, _) = load_retired_segment_rows(conn, expected.scope_id, expected.epoch)?;
    directory.verify_identity(scope_directory_identity)?;
    for (index, row) in rows.iter().enumerate() {
        let mut attempts = 0_usize;
        loop {
            attempts += 1;
            if attempts > 64 {
                return Err(Error::Conflict(format!(
                    "too many conflicting quarantine allocations for segment `{}`",
                    row.segment_id
                )));
            }
            let allocation = load_active_quarantine_allocation(
                conn,
                expected.scope_id,
                expected.epoch,
                &row.segment_id,
            )?;
            directory.verify_identity(allocation.scope_directory_identity)?;
            match allocation.state.as_str() {
                "allocated" => {
                    let opened = directory.open_regular(&allocation.quarantine_leaf)?;
                    let observed = super::secure_fs::file_identity(&opened)?;
                    if Some(observed) != allocation.quarantine_identity
                        || observed != allocation.source_identity
                    {
                        abandon_quarantine_allocation_and_replace(
                            conn,
                            expected,
                            &row.segment_id,
                            &allocation,
                            Some(observed),
                            "allocated_direct_quarantine_identity_mismatch",
                            false,
                        )?;
                        return Err(Error::InvalidInput(
                            "allocated direct quarantine identity changed".into(),
                        ));
                    }
                    authenticate_direct_quarantine(
                        &opened,
                        row,
                        expected.scope_id,
                        expected.epoch,
                        limits,
                    )?;
                    break;
                }
                "allocating" => match directory.open_regular(&allocation.quarantine_leaf) {
                    Ok(opened) => {
                        let observed = super::secure_fs::file_identity(&opened)?;
                        if observed != allocation.source_identity {
                            let source_still_present =
                                open_optional_regular(&directory, &row.segment_path)?.is_some();
                            abandon_quarantine_allocation_and_replace(
                                conn,
                                expected,
                                &row.segment_id,
                                &allocation,
                                Some(observed),
                                "direct_quarantine_target_identity_mismatch",
                                source_still_present,
                            )?;
                            if source_still_present {
                                continue;
                            }
                            return Err(Error::InvalidInput(
                                "direct quarantine target does not match journaled source".into(),
                            ));
                        }
                        authenticate_direct_quarantine(
                            &opened,
                            row,
                            expected.scope_id,
                            expected.epoch,
                            limits,
                        )?;
                        crate::db::util::test_crash_point(
                            "changed_path_deletion_after_direct_quarantine_verify",
                        );
                        opened.sync_all()?;
                        directory.sync()?;
                        crate::db::util::test_crash_point(
                            "changed_path_deletion_after_direct_quarantine_fsync",
                        );
                        mark_quarantine_allocation_allocated(conn, &allocation, observed)?;
                        break;
                    }
                    Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => {
                        let source = directory.open_regular(&row.segment_path)?;
                        if super::secure_fs::file_identity(&source)? != allocation.source_identity {
                            return Err(Error::InvalidInput(
                                "retirement source changed after allocation journal".into(),
                            ));
                        }
                        #[cfg(debug_assertions)]
                        run_deletion_substitution_hook(
                            DeletionSubstitutionPoint::BeforeQuarantineMove,
                        );
                        match directory
                            .rename_leaf_noreplace(&row.segment_path, &allocation.quarantine_leaf)
                        {
                            Ok(()) => {
                                crate::db::util::test_crash_point(
                                    "changed_path_deletion_after_direct_quarantine_rename",
                                );
                                #[cfg(debug_assertions)]
                                run_deletion_substitution_hook(
                                    DeletionSubstitutionPoint::AfterDirectRenameBeforeVerify,
                                );
                            }
                            Err(Error::Io(error))
                                if error.kind() == std::io::ErrorKind::AlreadyExists =>
                            {
                                let observed = directory
                                    .open_regular(&allocation.quarantine_leaf)
                                    .ok()
                                    .and_then(|opened| {
                                        super::secure_fs::file_identity(&opened).ok()
                                    });
                                abandon_quarantine_allocation_and_replace(
                                    conn,
                                    expected,
                                    &row.segment_id,
                                    &allocation,
                                    observed,
                                    "direct_quarantine_target_preexisting",
                                    true,
                                )?;
                            }
                            Err(error) => return Err(error),
                        }
                    }
                    Err(error) => return Err(error),
                },
                state => {
                    return Err(Error::Corrupt(format!(
                        "unexpected active quarantine allocation state `{state}`"
                    )));
                }
            }
        }
        if index + 1 < rows.len() {
            crate::db::util::test_crash_point("changed_path_deletion_between_allocation_segments");
        }
    }
    crate::db::util::test_crash_point("changed_path_deletion_after_allocation_setup");
    Ok(rows)
}

fn prepare_segment_deletion_transactions(
    tx: &Transaction<'_>,
    database_path: &std::path::Path,
    scope_id: super::ScopeId,
    epoch: u64,
    rows: &[RetiredSegmentRow],
) -> Result<()> {
    let (limits, persisted_rows) = load_retired_segment_rows(tx, scope_id, epoch)?;
    if persisted_rows.len() != rows.len() {
        return Err(Error::Corrupt(
            "retirement segment set changed after writer quiescence".into(),
        ));
    }
    if rows.is_empty() {
        return Ok(());
    }
    let trail_root = database_path
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or_else(|| Error::Corrupt("retirement database has no Trail root".into()))?;
    let directory_path = trail_root
        .join("observer-segments")
        .join(scope_id.to_text());
    let directory = super::secure_fs::SecureDirectory::open_absolute(&directory_path)?;
    let scope_directory_identity = directory.identity()?;
    let (retirement_generation, retirement_owner_token, retirement_fence): (i64, String, Vec<u8>) =
        tx.query_row(
            "SELECT scope.continuity_generation,owner.owner_token,owner.fence_nonce
         FROM changed_path_scopes scope
         JOIN changed_path_observer_owners owner ON owner.scope_id=scope.scope_id
         WHERE scope.scope_id=?1 AND scope.epoch=?2
           AND scope.trust_state='untrusted_gap' AND scope.trust_reason='scope_retiring'
           AND owner.epoch=scope.epoch AND owner.lease_state='revoked'",
            params![scope_id.to_text(), sql_u64(epoch, "scope epoch")?],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    if retirement_fence.len() != 32 {
        return Err(Error::Corrupt(
            "retirement owner fence nonce is not exact".into(),
        ));
    }
    let mut expected_previous_segment_id: Option<String> = None;
    let mut expected_previous_segment_hash = [0; 32];
    let mut expected_first_sequence = 1_u64;
    let mut saw_open_segment = false;
    for row in rows {
        if saw_open_segment
            || row.previous_segment_id != expected_previous_segment_id
            || row.previous_segment_hash != expected_previous_segment_hash
            || row.first_sequence != expected_first_sequence
        {
            return Err(Error::Corrupt(format!(
                "retired observer segment metadata lineage is not exact at `{}`",
                row.segment_id
            )));
        }
        let existing: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM changed_path_segment_deletions
             WHERE scope_id=?1 AND epoch=?2 AND segment_id=?3)",
            params![
                scope_id.to_text(),
                sql_u64(epoch, "scope epoch")?,
                row.segment_id
            ],
            |sql_row| sql_row.get(0),
        )?;
        if existing {
            return Err(Error::Corrupt(format!(
                "segment deletion transaction already exists before retirement: {}",
                row.segment_id
            )));
        }
        let allocation = load_active_quarantine_allocation(tx, scope_id, epoch, &row.segment_id)?;
        if hex::encode(row.owner_token) != retirement_owner_token {
            return Err(Error::Corrupt(
                "retirement segment owner does not match revoked fence owner".into(),
            ));
        }
        if allocation.state != "allocated" {
            return Err(Error::Corrupt(format!(
                "segment quarantine allocation is not ready for binding: {} state={}",
                row.segment_id, allocation.state
            )));
        }
        if allocation.scope_directory_identity != scope_directory_identity {
            return Err(Error::InvalidInput(format!(
                "segment quarantine allocation parent identity changed: {}",
                row.segment_id
            )));
        }
        let quarantine_identity = allocation
            .quarantine_identity
            .ok_or_else(|| Error::Corrupt("allocated quarantine identity is missing".into()))?;
        let file = directory.open_regular(&allocation.quarantine_leaf)?;
        let observed_identity = super::secure_fs::file_identity(&file)?;
        if observed_identity != quarantine_identity
            || observed_identity != allocation.source_identity
        {
            return Err(Error::InvalidInput(
                "direct quarantine identity changed before retirement binding".into(),
            ));
        }
        let segment_identity = allocation.source_identity;
        let authenticated = super::log::authenticate_segment_for_deletion(
            &file,
            &super::log::DeletionSegmentExpectation {
                scope_id,
                epoch,
                segment_id: row.segment_id.clone(),
                owner_token: row.owner_token,
                first_sequence: row.first_sequence,
                last_sequence: row.last_sequence,
                durable_end_offset: row.durable_end_offset,
                previous_segment_hash: row.previous_segment_hash,
                stored_segment_hash: row.stored_segment_hash,
                state: row.source_state.clone(),
                limits,
            },
        )
        .map_err(|error| Error::Corrupt(error.to_string()))?;
        expected_previous_segment_id = Some(row.segment_id.clone());
        expected_previous_segment_hash = authenticated.durable_hash;
        saw_open_segment = row.source_state == "open";
        expected_first_sequence = row
            .last_sequence
            .unwrap_or(row.first_sequence.saturating_sub(1))
            .checked_add(1)
            .ok_or_else(|| Error::Corrupt("retired segment sequence overflow".into()))?;
        let now = now_ts();
        tx.execute(
            "INSERT INTO changed_path_segment_deletions(
                 scope_id,epoch,segment_id,original_leaf,quarantine_leaf,allocation_nonce,
                 log_format_version,provider_id,folded_end_offset,
                 retirement_continuity_generation,retirement_fence_nonce,
                 scope_directory_device,scope_directory_inode,quarantine_device,quarantine_inode,
                 segment_device,segment_inode,file_length,file_hash,durable_end_offset,
                 durable_hash,max_observer_log_bytes,max_segment_bytes,
                 max_unfolded_tail_records,owner_token,first_sequence,last_sequence,
                 previous_segment_id,previous_segment_hash,source_state,state,
                 created_at,updated_at,completed_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,
                    ?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,
                    ?28,?29,?30,'quiesced',?31,?31,?31)",
            params![
                scope_id.to_text(),
                sql_u64(epoch, "scope epoch")?,
                row.segment_id,
                row.segment_path,
                allocation.quarantine_leaf,
                allocation.attempt_nonce,
                sql_u64(row.log_format_version, "segment log format")?,
                row.provider_id,
                sql_u64(row.folded_end_offset, "segment folded offset")?,
                retirement_generation,
                retirement_fence,
                encode_fs_identity_part(scope_directory_identity.0),
                encode_fs_identity_part(scope_directory_identity.1),
                encode_fs_identity_part(quarantine_identity.0),
                encode_fs_identity_part(quarantine_identity.1),
                encode_fs_identity_part(segment_identity.0),
                encode_fs_identity_part(segment_identity.1),
                sql_u64(authenticated.file_length, "segment file length")?,
                hex::encode(authenticated.file_hash),
                sql_u64(row.durable_end_offset, "segment durable offset")?,
                hex::encode(authenticated.durable_hash),
                sql_u64(limits.max_log_bytes, "maximum observer log bytes")?,
                sql_u64(limits.max_segment_bytes, "maximum segment bytes")?,
                sql_u64(
                    u64::try_from(limits.max_unfolded_tail_records).map_err(|_| {
                        Error::InvalidInput("maximum unfolded records exceeds SQLite".into())
                    })?,
                    "maximum unfolded records"
                )?,
                hex::encode(row.owner_token),
                sql_u64(row.first_sequence, "segment first sequence")?,
                row.last_sequence
                    .map(|value| sql_u64(value, "segment last sequence"))
                    .transpose()?,
                row.previous_segment_id,
                hex::encode(row.previous_segment_hash),
                row.source_state,
                now,
            ],
        )?;
        let bound = tx.execute(
            "UPDATE changed_path_segment_quarantine_allocations
             SET state='bound',updated_at=?1,bound_at=?1
             WHERE attempt_nonce=?2 AND state='allocated'",
            params![now, allocation.attempt_nonce],
        )?;
        if bound != 1 {
            return Err(Error::Conflict(format!(
                "segment quarantine allocation changed before binding: {}",
                row.segment_id
            )));
        }
    }
    directory.sync()?;
    Ok(())
}

fn validate_segment_deletion_transaction_coverage(
    conn: &Connection,
    scope_id: super::ScopeId,
    epoch: u64,
) -> Result<()> {
    let (segments, deletions, missing, invalid_allocations): (i64, i64, i64, i64) = conn
        .query_row(
            "SELECT
             (SELECT COUNT(*) FROM changed_path_observer_segments
              WHERE scope_id=?1 AND epoch=?2),
             (SELECT COUNT(*) FROM changed_path_segment_deletions
              WHERE scope_id=?1 AND epoch=?2),
             (SELECT COUNT(*) FROM changed_path_observer_segments segment
              WHERE segment.scope_id=?1 AND segment.epoch=?2
                AND NOT EXISTS(
                    SELECT 1 FROM changed_path_segment_deletions deletion
                    WHERE deletion.scope_id=segment.scope_id AND deletion.epoch=segment.epoch
                      AND deletion.segment_id=segment.segment_id)),
             (SELECT COUNT(*) FROM changed_path_segment_deletions deletion
              LEFT JOIN changed_path_segment_quarantine_allocations allocation
                ON allocation.attempt_nonce=deletion.allocation_nonce
              LEFT JOIN changed_path_observer_segments segment
                ON segment.scope_id=deletion.scope_id AND segment.epoch=deletion.epoch
               AND segment.segment_id=deletion.segment_id
              LEFT JOIN changed_path_scopes scope ON scope.scope_id=deletion.scope_id
              LEFT JOIN changed_path_observer_owners owner ON owner.scope_id=deletion.scope_id
              WHERE deletion.scope_id=?1 AND deletion.epoch=?2
                AND (allocation.attempt_nonce IS NULL OR allocation.state<>'bound'
                     OR segment.segment_id IS NULL OR segment.state<>'retired'
                     OR scope.retired_at IS NULL OR scope.trust_reason<>'scope_retired'
                     OR owner.owner_token IS NULL OR owner.lease_state<>'revoked'
                     OR allocation.scope_id<>deletion.scope_id
                     OR allocation.epoch<>deletion.epoch
                     OR allocation.segment_id<>deletion.segment_id
                     OR allocation.quarantine_leaf<>deletion.quarantine_leaf
                     OR allocation.scope_directory_device<>
                        deletion.scope_directory_device
                     OR allocation.scope_directory_inode<>
                        deletion.scope_directory_inode
                     OR allocation.source_segment_device<>deletion.segment_device
                     OR allocation.source_segment_inode<>deletion.segment_inode
                     OR allocation.quarantine_device<>deletion.quarantine_device
                     OR allocation.quarantine_inode<>deletion.quarantine_inode
                     OR deletion.original_leaf<>segment.segment_path
                     OR deletion.log_format_version<>segment.log_format_version
                     OR deletion.provider_id<>segment.provider_id
                     OR deletion.provider_id<>owner.provider_id
                     OR deletion.provider_id IS NOT scope.provider_id
                     OR deletion.owner_token<>segment.owner_token
                     OR deletion.owner_token<>owner.owner_token
                     OR deletion.first_sequence<>segment.first_sequence
                     OR deletion.last_sequence IS NOT segment.last_sequence
                     OR deletion.durable_end_offset<>segment.durable_end_offset
                     OR deletion.folded_end_offset<>segment.folded_end_offset
                     OR deletion.previous_segment_id IS NOT segment.previous_segment_id
                     OR deletion.previous_segment_hash<>
                        COALESCE(segment.previous_segment_hash,
                          '0000000000000000000000000000000000000000000000000000000000000000')
                     OR deletion.source_state<>segment.retirement_source_state
                     OR deletion.file_length<>segment.retirement_file_length
                     OR deletion.file_hash<>segment.retirement_file_hash
                     OR deletion.durable_hash<>segment.retirement_durable_hash
                     OR deletion.segment_device<>segment.retirement_source_device
                     OR deletion.segment_inode<>segment.retirement_source_inode
                     OR deletion.retirement_continuity_generation<>scope.continuity_generation
                     OR deletion.retirement_fence_nonce<>owner.fence_nonce))",
            params![scope_id.to_text(), sql_u64(epoch, "scope epoch")?],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )?;
    if segments != deletions || missing != 0 || invalid_allocations != 0 {
        return Err(Error::Corrupt(format!(
            "retired segment deletion transaction coverage mismatch: segments={segments} deletions={deletions} missing={missing} invalid_allocations={invalid_allocations}"
        )));
    }
    Ok(())
}

fn load_segment_deletion_tokens(
    conn: &Connection,
    database_path: &std::path::Path,
    scope_id: super::ScopeId,
    epoch: u64,
) -> Result<Vec<SegmentDeletionToken>> {
    validate_segment_deletion_transaction_coverage(conn, scope_id, epoch)?;
    let trail_root = database_path
        .parent()
        .and_then(std::path::Path::parent)
        .ok_or_else(|| Error::Corrupt("retirement database has no Trail root".into()))?;
    let rows = load_persisted_segment_deletions(conn, scope_id, epoch)?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let directory = std::sync::Arc::new(super::secure_fs::SecureDirectory::open_absolute(
        &trail_root
            .join("observer-segments")
            .join(scope_id.to_text()),
    )?);
    rows.into_iter()
        .map(|(segment_id, identity)| {
            directory.verify_identity(identity.scope_directory_identity)?;
            let quarantined = directory.open_regular(&identity.quarantine_leaf)?;
            if super::secure_fs::file_identity(&quarantined)? != identity.quarantine_identity {
                return Err(Error::InvalidInput(
                    "direct quarantine identity changed while loading deletion token".into(),
                ));
            }
            Ok(SegmentDeletionToken {
                scope_id,
                epoch,
                segment_id,
                directory: directory.clone(),
                identity,
            })
        })
        .collect()
}

pub(crate) fn remove_retired_segments(
    conn: &Connection,
    tokens: &[SegmentDeletionToken],
) -> Result<()> {
    for token in tokens {
        let current = load_one_persisted_segment_deletion(
            conn,
            token.scope_id,
            token.epoch,
            &token.segment_id,
        )?;
        validate_deletion_token_identity(token, &current)?;
        validate_retired_segment_paths(std::slice::from_ref(&current.original_leaf))?;
        token
            .directory
            .verify_identity(current.scope_directory_identity)?;
        #[cfg(debug_assertions)]
        run_deletion_substitution_hook(DeletionSubstitutionPoint::Parent);
        let original = open_optional_regular(&token.directory, &current.original_leaf)?;
        let quarantined = open_optional_regular(&token.directory, &current.quarantine_leaf)?;
        if current.state != "quiesced" || original.is_some() || quarantined.is_none() {
            return Err(Error::InvalidInput(
                "quiesced direct segment retirement has missing or reappeared evidence".into(),
            ));
        }
        let quarantined = quarantined.expect("checked direct quarantine presence");
        if super::secure_fs::file_identity(&quarantined)? != current.quarantine_identity {
            return Err(Error::InvalidInput(
                "quiesced direct quarantine inode changed".into(),
            ));
        }
        authenticate_persisted_deletion_file(token, &current, &quarantined)?;
    }
    Ok(())
}

fn load_persisted_segment_deletions(
    conn: &Connection,
    scope_id: super::ScopeId,
    epoch: u64,
) -> Result<Vec<(String, PersistedSegmentDeletion)>> {
    conn.prepare(
        "SELECT segment_id,original_leaf,quarantine_leaf,
                scope_directory_device,scope_directory_inode,quarantine_device,quarantine_inode,
                segment_device,segment_inode,file_length,file_hash,durable_end_offset,
                durable_hash,max_observer_log_bytes,max_segment_bytes,
                max_unfolded_tail_records,owner_token,first_sequence,last_sequence,
                previous_segment_id,previous_segment_hash,source_state,state
         FROM changed_path_segment_deletions
         WHERE scope_id=?1 AND epoch=?2 ORDER BY segment_id",
    )?
    .query_map(
        params![scope_id.to_text(), sql_u64(epoch, "scope epoch")?],
        decode_persisted_segment_deletion,
    )?
    .collect::<std::result::Result<Vec<_>, _>>()?
    .into_iter()
    .map(decode_persisted_segment_deletion_values)
    .collect()
}

type PersistedSegmentDeletionValues = (
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    String,
    i64,
    String,
    i64,
    i64,
    i64,
    String,
    i64,
    Option<i64>,
    Option<String>,
    String,
    String,
    String,
);

fn decode_persisted_segment_deletion(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<PersistedSegmentDeletionValues> {
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
        row.get(9)?,
        row.get(10)?,
        row.get(11)?,
        row.get(12)?,
        row.get(13)?,
        row.get(14)?,
        row.get(15)?,
        row.get(16)?,
        row.get(17)?,
        row.get(18)?,
        row.get(19)?,
        row.get(20)?,
        row.get(21)?,
        row.get(22)?,
    ))
}

fn decode_persisted_segment_deletion_values(
    row: PersistedSegmentDeletionValues,
) -> Result<(String, PersistedSegmentDeletion)> {
    Ok((
        row.0,
        PersistedSegmentDeletion {
            original_leaf: row.1,
            quarantine_leaf: row.2,
            scope_directory_identity: (
                decode_fs_identity_part(&row.3, "scope directory device")?,
                decode_fs_identity_part(&row.4, "scope directory inode")?,
            ),
            quarantine_identity: (
                decode_fs_identity_part(&row.5, "quarantine device")?,
                decode_fs_identity_part(&row.6, "quarantine inode")?,
            ),
            segment_identity: (
                decode_fs_identity_part(&row.7, "segment device")?,
                decode_fs_identity_part(&row.8, "segment inode")?,
            ),
            file_length: db_u64(row.9, "deletion file length")?,
            file_hash: decode_hex_32(&row.10, "deletion file hash")?,
            durable_end_offset: db_u64(row.11, "deletion durable offset")?,
            durable_hash: decode_hex_32(&row.12, "deletion durable hash")?,
            limits: super::PersistedLogLimits {
                max_log_bytes: db_u64(row.13, "deletion maximum log bytes")?,
                max_segment_bytes: db_u64(row.14, "deletion maximum segment bytes")?,
                max_unfolded_tail_records: usize::try_from(db_u64(
                    row.15,
                    "deletion maximum unfolded records",
                )?)
                .map_err(|_| Error::Corrupt("deletion record cap exceeds memory".into()))?,
            },
            owner_token: decode_hex_32(&row.16, "deletion owner token")?,
            first_sequence: db_u64(row.17, "deletion first sequence")?,
            last_sequence: row
                .18
                .map(|value| db_u64(value, "deletion last sequence"))
                .transpose()?,
            previous_segment_id: row.19,
            previous_segment_hash: decode_hex_32(&row.20, "deletion previous hash")?,
            source_state: row.21,
            state: row.22,
        },
    ))
}

fn load_one_persisted_segment_deletion(
    conn: &Connection,
    scope_id: super::ScopeId,
    epoch: u64,
    segment_id: &str,
) -> Result<PersistedSegmentDeletion> {
    load_persisted_segment_deletions(conn, scope_id, epoch)?
        .into_iter()
        .find_map(|(observed_id, identity)| (observed_id == segment_id).then_some(identity))
        .ok_or_else(|| {
            Error::Corrupt(format!(
                "persisted segment deletion disappeared: {}/{epoch}/{segment_id}",
                scope_id.to_text()
            ))
        })
}

fn validate_deletion_token_identity(
    token: &SegmentDeletionToken,
    current: &PersistedSegmentDeletion,
) -> Result<()> {
    let mut expected = token.identity.clone();
    expected.state.clone_from(&current.state);
    if expected != *current {
        return Err(Error::InvalidInput(
            "persisted segment deletion identity changed after authority issuance".into(),
        ));
    }
    Ok(())
}

fn open_optional_regular(
    directory: &super::secure_fs::SecureDirectory,
    leaf: &str,
) -> Result<Option<std::fs::File>> {
    match directory.open_regular(leaf) {
        Ok(file) => Ok(Some(file)),
        Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn authenticate_persisted_deletion_file(
    token: &SegmentDeletionToken,
    identity: &PersistedSegmentDeletion,
    file: &std::fs::File,
) -> Result<()> {
    if super::secure_fs::file_identity(file)? != identity.segment_identity {
        return Err(Error::InvalidInput(
            "segment deletion inode does not match persisted authority".into(),
        ));
    }
    let authenticated = super::log::authenticate_segment_for_deletion(
        file,
        &super::log::DeletionSegmentExpectation {
            scope_id: token.scope_id,
            epoch: token.epoch,
            segment_id: token.segment_id.clone(),
            owner_token: identity.owner_token,
            first_sequence: identity.first_sequence,
            last_sequence: identity.last_sequence,
            durable_end_offset: identity.durable_end_offset,
            previous_segment_hash: identity.previous_segment_hash,
            stored_segment_hash: (identity.source_state == "sealed")
                .then_some(identity.durable_hash),
            state: identity.source_state.clone(),
            limits: identity.limits,
        },
    )
    .map_err(|error| Error::Corrupt(error.to_string()))?;
    if authenticated.file_length != identity.file_length
        || authenticated.file_hash != identity.file_hash
        || authenticated.durable_hash != identity.durable_hash
    {
        return Err(Error::InvalidInput(
            "segment deletion bytes do not match persisted authority".into(),
        ));
    }
    Ok(())
}

fn update_segment_deletion_state(
    conn: &Connection,
    token: &SegmentDeletionToken,
    expected_states: &[&str],
    state: &str,
    completed: bool,
) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let persisted =
        load_one_persisted_segment_deletion(&tx, token.scope_id, token.epoch, &token.segment_id)?;
    validate_deletion_token_identity(token, &persisted)?;
    let current = persisted.state;
    if current == state || (completed && current == "quiesced") {
        tx.commit()?;
        durable_intent_barrier(conn)?;
        return Ok(());
    }
    if !expected_states.contains(&current.as_str()) {
        return Err(Error::InvalidInput(format!(
            "segment deletion state `{current}` cannot advance to `{state}`"
        )));
    }
    let now = now_ts();
    let changed = tx.execute(
        "UPDATE changed_path_segment_deletions
         SET state=?1,updated_at=?2,completed_at=?3
         WHERE scope_id=?4 AND epoch=?5 AND segment_id=?6 AND state=?7",
        params![
            state,
            now,
            completed.then_some(now),
            token.scope_id.to_text(),
            sql_u64(token.epoch, "scope epoch")?,
            token.segment_id,
            current,
        ],
    )?;
    if changed != 1 {
        return Err(Error::Conflict(
            "segment deletion state changed during durable transition".into(),
        ));
    }
    tx.commit()?;
    durable_intent_barrier(conn)
}

fn encode_fs_identity_part(value: u64) -> String {
    format!("{value:016x}")
}

fn decode_fs_identity_part(value: &str, label: &str) -> Result<u64> {
    u64::from_str_radix(value, 16).map_err(|_| Error::Corrupt(format!("invalid persisted {label}")))
}

fn decode_hex_32(value: &str, label: &str) -> Result<[u8; 32]> {
    hex::decode(value)
        .map_err(|_| Error::Corrupt(format!("invalid {label} encoding")))?
        .try_into()
        .map_err(|_| Error::Corrupt(format!("invalid {label} length")))
}

fn finish_publication(
    tx: &Transaction<'_>,
    intent: &PersistedIntent,
    expected: &ExpectedScope,
) -> Result<bool> {
    let changed = tx.execute(
        "UPDATE changed_path_scopes SET change_id=?1,baseline_root_id=?2,
             ref_generation=?3,updated_at=?4
         WHERE scope_id=?5 AND epoch=?6 AND ref_name=?7 AND ref_generation=?8
           AND change_id=?9 AND baseline_root_id=?10 AND trust_state='trusted'",
        params![
            intent.target.change_id.0,
            intent.target.root_id.0,
            sql_u64(
                intent.expected_ref_generation.saturating_add(1),
                "target generation"
            )?,
            now_ts(),
            intent.scope_id,
            sql_u64(intent.expected_epoch, "scope epoch")?,
            intent.expected_ref_name,
            sql_u64(intent.expected_ref_generation, "ref generation")?,
            intent.expected_change_id.0,
            intent.expected_root_id.0
        ],
    )?;
    if changed != 1 {
        retain_and_fail_closed(tx, intent, "intent_baseline_publication_cas_failed")?;
        return Ok(false);
    }
    clear_intent_ownership(tx, intent)?;
    let acknowledged = tx.execute(
        "UPDATE changed_path_intents SET lifecycle_state='acknowledged',failure_reason=NULL,
             updated_at=?1 WHERE intent_id=?2 AND lifecycle_state='published'",
        params![now_ts(), intent.id.0],
    )?;
    if acknowledged != 1 {
        return Err(Error::Corrupt(format!(
            "published intent `{}` could not be acknowledged",
            intent.id.0
        )));
    }
    let _ = expected;
    Ok(true)
}

fn clear_intent_ownership(tx: &Transaction<'_>, intent: &PersistedIntent) -> Result<()> {
    for table in ["changed_path_entries", "changed_path_prefixes"] {
        let delete = format!("DELETE FROM {table} WHERE intent_id=?1 AND source_mask=2");
        tx.execute(&delete, [&intent.id.0])?;
        let retain = format!(
            "UPDATE {table} SET source_mask=source_mask & ~2,intent_id=NULL,updated_at=?1
             WHERE intent_id=?2 AND (source_mask & ~2) != 0"
        );
        tx.execute(&retain, params![now_ts(), intent.id.0])?;
    }
    Ok(())
}

fn retain_and_fail_closed(
    tx: &Transaction<'_>,
    intent: &PersistedIntent,
    reason: &str,
) -> Result<()> {
    stage_intent_evidence(tx, intent)?;
    tx.execute(
        "UPDATE changed_path_intents SET lifecycle_state='aborted',failure_reason=?1,
             updated_at=?2 WHERE intent_id=?3 AND lifecycle_state IN
             ('prepared','filesystem_applied','published')",
        params![reason, now_ts(), intent.id.0],
    )?;
    tx.execute(
        "UPDATE changed_path_scopes SET
             continuity_generation=continuity_generation+
                 CASE WHEN trust_state IN ('trusted','reconciling') THEN 1 ELSE 0 END,
             trust_state=CASE WHEN trust_state IN ('trusted','reconciling')
                              THEN 'untrusted_gap' ELSE trust_state END,
             trust_reason=CASE WHEN trust_state IN ('trusted','reconciling')
                               THEN ?1 ELSE trust_reason END,
             updated_at=?2 WHERE scope_id=?3",
        params![reason, now_ts(), intent.scope_id],
    )?;
    Ok(())
}

fn pending_intent_ids(conn: &Connection, scope_id: &str) -> Result<Vec<IntentId>> {
    conn.prepare(
        "SELECT intent_id FROM changed_path_intents WHERE scope_id=?1
         AND lifecycle_state IN ('prepared','filesystem_applied','published')
         ORDER BY created_at,intent_id",
    )?
    .query_map([scope_id], |row| row.get::<_, String>(0))?
    .map(|row| row.map(IntentId).map_err(Error::from))
    .collect()
}

fn intent_matches_expected_scope(intent: &PersistedIntent, expected: &ExpectedScope) -> bool {
    intent.scope_id == expected.scope_id.to_text()
        && intent.expected_epoch == expected.epoch
        && intent.expected_ref_name == expected.ref_name
        && intent.expected_ref_generation == expected.ref_generation
        && intent.expected_root_id == expected.baseline_root
        && intent
            .verified_cut
            .as_ref()
            .is_none_or(|proof| proof.filesystem_identity == expected.filesystem_identity)
}

fn current_change_matches(conn: &Connection, intent: &PersistedIntent) -> Result<bool> {
    let change = conn
        .query_row(
            "SELECT change_id FROM changed_path_scopes WHERE scope_id=?1",
            [&intent.scope_id],
            |row| row.get::<_, String>(0),
        )
        .optional()?;
    Ok(change.as_deref() == Some(intent.expected_change_id.0.as_str()))
}

fn scope_cas_matches(conn: &Connection, expected: &ExpectedScope) -> Result<bool> {
    match super::intent::exact_scope_guard(conn, expected, false) {
        Ok(()) => Ok(true),
        Err(Error::ChangeLedgerReconcileRequired { .. }) => Ok(false),
        Err(error) => Err(error),
    }
}

fn scopes_with_pending_intents(conn: &Connection) -> Result<Vec<ExpectedScope>> {
    let mut statement = conn.prepare(
        "SELECT DISTINCT s.scope_id,s.epoch,s.ref_name,s.ref_generation,s.baseline_root_id,
                s.policy_fingerprint,s.policy_dependency_generation,s.filesystem_identity,
                COALESCE(s.provider_identity,'')
         FROM changed_path_scopes s JOIN changed_path_intents i ON i.scope_id=s.scope_id
         WHERE i.lifecycle_state IN ('prepared','filesystem_applied','published')
         ORDER BY s.scope_id",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, String>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, String>(7)?,
            row.get::<_, String>(8)?,
        ))
    })?;
    rows.map(|row| {
        let row = row?;
        let scope_bytes = hex::decode(row.0).map_err(|error| Error::Corrupt(error.to_string()))?;
        let scope_id: [u8; 32] = scope_bytes
            .try_into()
            .map_err(|_| Error::Corrupt("invalid scope id".into()))?;
        let policy_bytes = hex::decode(row.5).map_err(|error| Error::Corrupt(error.to_string()))?;
        let policy: [u8; 32] = policy_bytes
            .try_into()
            .map_err(|_| Error::Corrupt("invalid policy fingerprint".into()))?;
        Ok(ExpectedScope {
            scope_id: super::ScopeId(scope_id),
            epoch: db_u64(row.1, "scope epoch")?,
            ref_name: row.2,
            ref_generation: db_u64(row.3, "ref generation")?,
            baseline_root: ObjectId(row.4),
            policy_fingerprint: policy,
            policy_generation: db_u64(row.6, "policy generation")?,
            filesystem_identity: hex::decode(row.7)
                .map_err(|error| Error::Corrupt(error.to_string()))?,
            provider_identity: hex::decode(row.8)
                .map_err(|error| Error::Corrupt(error.to_string()))?,
        })
    })
    .collect()
}

#[cfg(debug_assertions)]
mod harness {
    use rusqlite::{params, Connection};
    use std::fs::OpenOptions;
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    use super::*;
    use crate::db::change_ledger::intent::install_sidecar_ancestor_substitution_hook;
    use crate::db::change_ledger::{
        mark_filesystem_applied, prepare_intent, publish_intent, BaselineIdentity, DirtyPrefix,
        DurableCut, EvidenceCut, EvidenceFlags, EvidenceSource, FilesystemIdentity, IntentEvidence,
        IntentProducer, IntentTarget, LedgerPath, PolicyIdentity, ProviderCapabilities,
        ProviderIdentity, QualifiedFilesystemProof, ScopeId, ScopeIdentity, ScopeKind,
    };
    use crate::db::{InitImportMode, Trail};
    use crate::model::{Actor, FileContentRef};
    use crate::{ChangeId, ObjectId};

    struct Fixture {
        _root: tempfile::TempDir,
        db: Trail,
        expected: ExpectedScope,
        backup: std::path::PathBuf,
        restore: std::path::PathBuf,
    }

    impl Fixture {
        fn new(tag: u8) -> Result<Self> {
            let root = tempfile::tempdir()?;
            let workspace = root.path().join("workspace");
            std::fs::create_dir_all(&workspace)?;
            Trail::init(&workspace, "main", InitImportMode::Empty, false)?;
            let db = Trail::open(&workspace)?;
            let head = db.get_ref("refs/branches/main")?;
            let scope = ScopeIdentity {
                scope_id: ScopeId([tag; 32]),
                kind: ScopeKind::Workspace,
                owner_id: format!("recovery-harness-{tag}"),
            };
            let baseline = BaselineIdentity {
                ref_name: head.name.clone(),
                ref_generation: head
                    .generation
                    .try_into()
                    .map_err(|_| Error::Corrupt("negative fixture generation".into()))?,
                change_id: head.change_id.clone(),
                root_id: head.root_id.clone(),
            };
            let policy = PolicyIdentity {
                fingerprint: [tag.wrapping_add(1); 32],
                generation: 1,
            };
            let filesystem = FilesystemIdentity(vec![tag, 0, 0xff]);
            let provider = ProviderIdentity {
                identity: vec![tag.wrapping_add(2), 0x80],
                capabilities: ProviderCapabilities {
                    durable_cursor: true,
                    linearizable_fence: true,
                    rename_pairing: true,
                    overflow_scope: true,
                    filesystem_supported: true,
                    clean_proof_allowed: true,
                    power_loss_durability: true,
                },
            };
            db.changed_path_ledger().begin_scope(
                &scope,
                &baseline,
                &policy,
                &filesystem,
                &provider,
            )?;
            db.conn.execute(
                "UPDATE changed_path_scopes SET trust_state='trusted',trust_reason='test_fence',
                     provider_cursor=?1 WHERE scope_id=?2",
                params![b"cursor-1".as_slice(), scope.scope_id.to_text()],
            )?;
            let expected = ExpectedScope {
                scope_id: scope.scope_id,
                epoch: 1,
                ref_name: baseline.ref_name,
                ref_generation: baseline.ref_generation,
                baseline_root: baseline.root_id,
                policy_fingerprint: policy.fingerprint,
                policy_generation: policy.generation,
                filesystem_identity: filesystem.0,
                provider_identity: provider.identity,
            };
            let backup = root.path().join("backup");
            let restore = root.path().join("restore");
            Ok(Self {
                _root: root,
                db,
                expected,
                backup,
                restore,
            })
        }

        fn target(&self, suffix: &str, root: ObjectId) -> IntentTarget {
            IntentTarget {
                change_id: ChangeId(format!("change-recovery-{suffix}")),
                root_id: root,
                operation_id: None,
            }
        }

        fn evidence(path: &str) -> IntentEvidence {
            IntentEvidence {
                exact_paths: vec![LedgerPath::parse(path).unwrap()],
                complete_prefixes: Vec::new(),
            }
        }

        fn qualified_proof(&self, sequence: u64, path: &str) -> Result<QualifiedFilesystemProof> {
            let (proof, writer) = self.qualified_proof_with_writer(sequence, path)?;
            drop(writer);
            Ok(proof)
        }

        fn qualified_proof_with_writer(
            &self,
            sequence: u64,
            path: &str,
        ) -> Result<(QualifiedFilesystemProof, super::super::SegmentWriter)> {
            let provider_id: String = self.db.conn.query_row(
                "SELECT provider_id FROM changed_path_scopes WHERE scope_id=?1",
                [self.expected.scope_id.to_text()],
                |row| row.get(0),
            )?;
            let owner_token_bytes = [self.expected.scope_id.0[0].wrapping_add(0x31); 32];
            let owner_token = hex::encode(owner_token_bytes);
            let fence_nonce = b"qualified-fence".to_vec();
            let relative_directory =
                format!("observer-segments/{}", self.expected.scope_id.to_text());
            let segment_directory = self.db.db_dir.join(&relative_directory);
            let mut writer = super::super::SegmentWriter::acquire(
                &self.db.db_dir.join(crate::db::DB_RELATIVE_PATH),
                &segment_directory,
                self.expected.scope_id,
                self.expected.epoch,
                owner_token_bytes,
                &provider_id,
                b"cursor-1".to_vec(),
                Duration::from_secs(600),
            )?;
            let records = (1..=sequence)
                .map(|record_sequence| {
                    Ok(super::super::ObserverRecord {
                        sequence: record_sequence,
                        source: EvidenceSource::Observer,
                        path: LedgerPath::parse(path)?,
                        flags: EvidenceFlags::CONTENT,
                        provider_cursor: format!("cursor-{}", record_sequence + 1).into_bytes(),
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            writer.append(&records)?;
            let durable = writer.flush_durable()?;
            writer.rotate()?;
            let end_cursor = durable.provider_cursor.clone();
            self.insert_observer_event(path, sequence)?;
            self.db.conn.execute(
                "UPDATE changed_path_scopes SET provider_cursor=?1,durable_offset=?2,
                     folded_offset=?2 WHERE scope_id=?3",
                params![
                    end_cursor,
                    sql_u64(durable.durable_end_offset, "fixture durable offset")?,
                    self.expected.scope_id.to_text()
                ],
            )?;
            self.db.conn.execute(
                "UPDATE changed_path_observer_owners
                 SET provider_identity=?1,fence_nonce=?2 WHERE scope_id=?3",
                params![
                    hex::encode(&self.expected.provider_identity),
                    fence_nonce,
                    self.expected.scope_id.to_text(),
                ],
            )?;
            self.db.conn.execute(
                "UPDATE changed_path_observer_segments SET folded_end_offset=durable_end_offset
                 WHERE scope_id=?1 AND segment_id=?2",
                params![self.expected.scope_id.to_text(), durable.segment_id],
            )?;
            let segment = self.db.conn.query_row(
                "SELECT segment_hash,segment_path,durable_end_offset,folded_end_offset
                 FROM changed_path_observer_segments
                 WHERE scope_id=?1 AND segment_id=?2 AND state='sealed'",
                params![self.expected.scope_id.to_text(), durable.segment_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )?;
            let segment_hash: [u8; 32] = hex::decode(segment.0)
                .map_err(|error| Error::Corrupt(error.to_string()))?
                .try_into()
                .map_err(|_| Error::Corrupt("invalid fixture segment hash".into()))?;
            Ok((
                QualifiedFilesystemProof {
                    scope_id: self.expected.scope_id,
                    epoch: self.expected.epoch,
                    expected_root_id: self.expected.baseline_root.clone(),
                    scope_root_identity: self.expected.filesystem_identity.clone(),
                    filesystem_identity: self.expected.filesystem_identity.clone(),
                    provider_id,
                    provider_identity: self.expected.provider_identity.clone(),
                    observer_owner_token: owner_token,
                    owner_fence_nonce: Some(fence_nonce),
                    durable_segment_id: durable.segment_id,
                    durable_segment_hash: segment_hash,
                    segment_directory: relative_directory,
                    segment_path: segment.1,
                    start_cursor: Some(b"cursor-1".to_vec()),
                    end_cursor,
                    start_sequence: 1,
                    end_cut: EvidenceCut {
                        source: EvidenceSource::Observer,
                        sequence,
                        durable_offset: durable.durable_end_offset,
                        folded_offset: durable.durable_end_offset,
                    },
                    segment_durable_offset: db_u64(segment.2, "fixture segment durable")?,
                    segment_folded_offset: db_u64(segment.3, "fixture segment folded")?,
                    verified_paths: 1,
                    verified_prefixes: 0,
                    complete_root_interval: true,
                    complete_policy_interval: true,
                    persisted_evidence_through_end: true,
                },
                writer,
            ))
        }

        fn advance_ref_to(&self, target: &IntentTarget) -> Result<()> {
            self.db.conn.execute(
                "UPDATE refs SET change_id=?1,root_id=?2,generation=generation+1,updated_at=?3
                 WHERE name=?4 AND generation=?5",
                params![
                    target.change_id.0,
                    target.root_id.0,
                    now_ts(),
                    self.expected.ref_name,
                    sql_u64(self.expected.ref_generation, "fixture generation")?
                ],
            )?;
            Ok(())
        }

        fn insert_observer_event(&self, path: &str, sequence: u64) -> Result<()> {
            self.db.conn.execute(
                "INSERT INTO changed_path_entries(scope_id,normalized_path,event_flags,source_mask,
                     first_sequence,last_sequence,provider_id,provider_sequence,intent_id,created_at,updated_at)
                 VALUES(?1,?2,?3,1,?4,?4,'observer',?4,NULL,?5,?5)
                 ON CONFLICT(scope_id,normalized_path) DO UPDATE SET
                    event_flags=changed_path_entries.event_flags|excluded.event_flags,
                    source_mask=changed_path_entries.source_mask|excluded.source_mask,
                    last_sequence=MAX(changed_path_entries.last_sequence,excluded.last_sequence),
                    provider_sequence=MAX(changed_path_entries.provider_sequence,excluded.provider_sequence),
                    updated_at=excluded.updated_at",
                params![self.expected.scope_id.to_text(),path,EvidenceFlags::CONTENT.0,
                    sql_u64(sequence,"observer sequence")?,now_ts()],
            )?;
            Ok(())
        }

        fn insert_observer_prefix(&self, path: &str, sequence: u64) -> Result<()> {
            self.db.conn.execute(
                "INSERT INTO changed_path_prefixes(
                     scope_id,normalized_prefix,completeness_reason,event_flags,source_mask,
                     first_sequence,last_sequence,provider_id,provider_sequence,intent_id,
                     created_at,updated_at)
                 VALUES(?1,?2,'provider_complete',?3,1,?4,?4,'observer',?4,NULL,?5,?5)
                 ON CONFLICT(scope_id,normalized_prefix) DO UPDATE SET
                    event_flags=changed_path_prefixes.event_flags|excluded.event_flags,
                    source_mask=changed_path_prefixes.source_mask|excluded.source_mask,
                    first_sequence=MIN(changed_path_prefixes.first_sequence,excluded.first_sequence),
                    last_sequence=MAX(changed_path_prefixes.last_sequence,excluded.last_sequence),
                    provider_sequence=MAX(changed_path_prefixes.provider_sequence,excluded.provider_sequence),
                    updated_at=excluded.updated_at",
                params![
                    self.expected.scope_id.to_text(),
                    path,
                    EvidenceFlags::CONTENT.0,
                    sql_u64(sequence, "observer prefix sequence")?,
                    now_ts()
                ],
            )?;
            Ok(())
        }

        fn proof_for_durable_segment(
            &self,
            durable: &DurableCut,
            start_cursor: Vec<u8>,
            start_sequence: u64,
            verified_paths: u64,
            verified_prefixes: u64,
        ) -> Result<QualifiedFilesystemProof> {
            let provider_id: String = self.db.conn.query_row(
                "SELECT provider_id FROM changed_path_scopes WHERE scope_id=?1",
                [self.expected.scope_id.to_text()],
                |row| row.get(0),
            )?;
            let segment = self.db.conn.query_row(
                "SELECT segment_hash,segment_path,durable_end_offset,folded_end_offset
                 FROM changed_path_observer_segments
                 WHERE scope_id=?1 AND segment_id=?2 AND state='sealed'",
                params![self.expected.scope_id.to_text(), durable.segment_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )?;
            let segment_hash = hex::decode(segment.0)
                .map_err(|error| Error::Corrupt(error.to_string()))?
                .try_into()
                .map_err(|_| Error::Corrupt("invalid fixture segment hash".into()))?;
            Ok(QualifiedFilesystemProof {
                scope_id: self.expected.scope_id,
                epoch: self.expected.epoch,
                expected_root_id: self.expected.baseline_root.clone(),
                scope_root_identity: self.expected.filesystem_identity.clone(),
                filesystem_identity: self.expected.filesystem_identity.clone(),
                provider_id,
                provider_identity: self.expected.provider_identity.clone(),
                observer_owner_token: hex::encode(
                    [self.expected.scope_id.0[0].wrapping_add(0x31); 32],
                ),
                owner_fence_nonce: Some(b"qualified-fence".to_vec()),
                durable_segment_id: durable.segment_id.clone(),
                durable_segment_hash: segment_hash,
                segment_directory: format!(
                    "observer-segments/{}",
                    self.expected.scope_id.to_text()
                ),
                segment_path: segment.1,
                start_cursor: Some(start_cursor),
                end_cursor: durable.provider_cursor.clone(),
                start_sequence,
                end_cut: EvidenceCut {
                    source: EvidenceSource::Observer,
                    sequence: durable.last_sequence,
                    durable_offset: durable.durable_end_offset,
                    folded_offset: durable.durable_end_offset,
                },
                segment_durable_offset: db_u64(segment.2, "fixture segment durable")?,
                segment_folded_offset: db_u64(segment.3, "fixture segment folded")?,
                verified_paths,
                verified_prefixes,
                complete_root_interval: true,
                complete_policy_interval: true,
                persisted_evidence_through_end: true,
            })
        }
    }

    pub(super) fn acknowledgement_race() -> Result<()> {
        let fixture = Fixture::new(0x61)?;
        let target = fixture.target("ack", fixture.expected.baseline_root.clone());
        let ledger = fixture.db.changed_path_ledger();
        let intent = prepare_intent(
            &ledger,
            &fixture.expected,
            IntentProducer::Checkout,
            &target,
            &Fixture::evidence("src/lib.rs"),
        )?;
        mark_filesystem_applied(
            &ledger,
            &fixture.expected,
            &intent,
            &fixture.qualified_proof(7, "src/lib.rs")?,
        )?;
        fixture.insert_observer_event("src/lib.rs", 8)?;
        fixture.advance_ref_to(&target)?;
        publish_intent(&ledger, &fixture.expected, &intent)?;
        let decisions = recover_scope(&ledger, &fixture.expected)?;
        if decisions != vec![(intent.clone(), RecoveryDecision::FinishPublication)] {
            return Err(Error::Corrupt(format!(
                "unexpected recovery decision: {decisions:?}"
            )));
        }
        let row = fixture.db.conn.query_row(
            "SELECT source_mask,provider_sequence,intent_id FROM changed_path_entries
             WHERE scope_id=?1 AND normalized_path='src/lib.rs'",
            [fixture.expected.scope_id.to_text()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )?;
        if row != (EvidenceSource::Observer.mask(), Some(8), None) {
            return Err(Error::Corrupt(format!(
                "concurrent observer evidence was cleared: {row:?}"
            )));
        }
        Ok(())
    }

    pub(super) fn authenticated_prefix_survives_later_observer_advance() -> Result<()> {
        let fixture = Fixture::new(0x81)?;
        let target = fixture.target("advanced-prefix", fixture.expected.baseline_root.clone());
        let ledger = fixture.db.changed_path_ledger();
        let intent = prepare_intent(
            &ledger,
            &fixture.expected,
            IntentProducer::Checkout,
            &target,
            &Fixture::evidence("advanced-prefix.rs"),
        )?;
        let (proof, mut writer) = fixture.qualified_proof_with_writer(1, "advanced-prefix.rs")?;
        mark_filesystem_applied(&ledger, &fixture.expected, &intent, &proof)?;

        writer.append(&[super::super::ObserverRecord {
            sequence: 2,
            source: EvidenceSource::Observer,
            path: LedgerPath::parse("advanced-prefix.rs")?,
            flags: EvidenceFlags::CONTENT,
            provider_cursor: b"cursor-3".to_vec(),
        }])?;
        let advanced = writer.flush_durable()?;
        writer.rotate()?;
        drop(writer);
        fixture.db.conn.execute(
            "UPDATE changed_path_observer_segments SET folded_end_offset=durable_end_offset
             WHERE scope_id=?1 AND segment_id=?2",
            params![fixture.expected.scope_id.to_text(), advanced.segment_id],
        )?;
        fixture.db.conn.execute(
            "UPDATE changed_path_scopes SET provider_cursor=?1,durable_offset=?2,folded_offset=?2
             WHERE scope_id=?3",
            params![
                advanced.provider_cursor,
                sql_u64(advanced.durable_end_offset, "advanced durable offset")?,
                fixture.expected.scope_id.to_text(),
            ],
        )?;
        fixture.insert_observer_event("advanced-prefix.rs", 2)?;
        durable_intent_barrier(&fixture.db.conn)?;
        fixture.advance_ref_to(&target)?;
        publish_intent(&ledger, &fixture.expected, &intent)?;

        let decisions = recover_scope(&ledger, &fixture.expected)?;
        if decisions != vec![(intent.clone(), RecoveryDecision::FinishPublication)] {
            return Err(Error::Corrupt(format!(
                "authenticated prefix was not accepted after later observer advance: {decisions:?}"
            )));
        }
        let later_sequence: i64 = fixture.db.conn.query_row(
            "SELECT provider_sequence FROM changed_path_entries
             WHERE scope_id=?1 AND normalized_path='advanced-prefix.rs'",
            [fixture.expected.scope_id.to_text()],
            |row| row.get(0),
        )?;
        if later_sequence != 2 {
            return Err(Error::Corrupt(
                "later observer evidence was cleared with the intent prefix".into(),
            ));
        }
        Ok(())
    }

    fn aggregate_bridge_without_interval_evidence_is_rejected(prefix: bool) -> Result<()> {
        let fixture = Fixture::new(if prefix { 0x83 } else { 0x82 })?;
        let bridge_path = if prefix {
            "bridged-prefix"
        } else {
            "bridged-path.rs"
        };
        let target = fixture.target("aggregate-bridge", fixture.expected.baseline_root.clone());
        let owner_token = [fixture.expected.scope_id.0[0].wrapping_add(0x31); 32];
        let provider_id: String = fixture.db.conn.query_row(
            "SELECT provider_id FROM changed_path_scopes WHERE scope_id=?1",
            [fixture.expected.scope_id.to_text()],
            |row| row.get(0),
        )?;
        let segment_directory = fixture
            .db
            .db_dir
            .join("observer-segments")
            .join(fixture.expected.scope_id.to_text());
        let mut writer = super::super::SegmentWriter::acquire(
            &fixture.db.db_dir.join(crate::db::DB_RELATIVE_PATH),
            &segment_directory,
            fixture.expected.scope_id,
            fixture.expected.epoch,
            owner_token,
            &provider_id,
            b"cursor-1".to_vec(),
            Duration::from_secs(600),
        )?;
        writer.append(&[super::super::ObserverRecord {
            sequence: 1,
            source: EvidenceSource::Observer,
            path: LedgerPath::parse(bridge_path)?,
            flags: EvidenceFlags::CONTENT,
            provider_cursor: b"cursor-2".to_vec(),
        }])?;
        let before = writer.flush_durable()?;
        writer.rotate()?;
        if prefix {
            fixture.insert_observer_prefix(bridge_path, 1)?;
        } else {
            fixture.insert_observer_event(bridge_path, 1)?;
        }
        fixture.db.conn.execute(
            "UPDATE changed_path_observer_segments SET folded_end_offset=durable_end_offset
             WHERE scope_id=?1 AND segment_id=?2",
            params![fixture.expected.scope_id.to_text(), before.segment_id],
        )?;
        fixture.db.conn.execute(
            "UPDATE changed_path_scopes SET provider_cursor=?1,durable_offset=?2,folded_offset=?2
             WHERE scope_id=?3",
            params![
                before.provider_cursor,
                sql_u64(before.durable_end_offset, "pre-intent durable offset")?,
                fixture.expected.scope_id.to_text()
            ],
        )?;

        let evidence = if prefix {
            IntentEvidence {
                exact_paths: Vec::new(),
                complete_prefixes: vec![DirtyPrefix {
                    path: LedgerPath::parse(bridge_path)?,
                    complete: true,
                    reason: "provider_complete".into(),
                    first_sequence: 2,
                    last_sequence: 2,
                }],
            }
        } else {
            Fixture::evidence(bridge_path)
        };
        let ledger = fixture.db.changed_path_ledger();
        let intent = prepare_intent(
            &ledger,
            &fixture.expected,
            IntentProducer::Checkout,
            &target,
            &evidence,
        )?;

        writer.append(&[super::super::ObserverRecord {
            sequence: 2,
            source: EvidenceSource::Observer,
            path: LedgerPath::parse("unrelated-during-intent.rs")?,
            flags: EvidenceFlags::CONTENT,
            provider_cursor: b"cursor-3".to_vec(),
        }])?;
        let proof_cut = writer.flush_durable()?;
        writer.rotate()?;
        writer.append(&[super::super::ObserverRecord {
            sequence: 3,
            source: EvidenceSource::Observer,
            path: LedgerPath::parse(bridge_path)?,
            flags: EvidenceFlags::CONTENT,
            provider_cursor: b"cursor-4".to_vec(),
        }])?;
        let after = writer.flush_durable()?;
        writer.rotate()?;
        drop(writer);
        fixture.db.conn.execute(
            "UPDATE changed_path_observer_segments SET folded_end_offset=durable_end_offset
             WHERE scope_id=?1",
            [fixture.expected.scope_id.to_text()],
        )?;
        if prefix {
            fixture.insert_observer_prefix(bridge_path, 3)?;
        } else {
            fixture.insert_observer_event(bridge_path, 3)?;
        }
        fixture.db.conn.execute(
            "UPDATE changed_path_observer_owners
             SET provider_identity=?1,fence_nonce=?2 WHERE scope_id=?3",
            params![
                hex::encode(&fixture.expected.provider_identity),
                b"qualified-fence".as_slice(),
                fixture.expected.scope_id.to_text(),
            ],
        )?;
        fixture.db.conn.execute(
            "UPDATE changed_path_scopes SET provider_cursor=?1,durable_offset=?2,folded_offset=?2
             WHERE scope_id=?3",
            params![
                after.provider_cursor,
                sql_u64(
                    after.durable_end_offset.max(proof_cut.durable_end_offset),
                    "post-intent durable offset"
                )?,
                fixture.expected.scope_id.to_text(),
            ],
        )?;
        let proof = fixture.proof_for_durable_segment(
            &proof_cut,
            b"cursor-2".to_vec(),
            2,
            u64::from(!prefix),
            u64::from(prefix),
        )?;

        if mark_filesystem_applied(&ledger, &fixture.expected, &intent, &proof).is_ok() {
            return Err(Error::Corrupt(format!(
                "{} SQL aggregate bridged an authenticated interval with no matching record",
                if prefix { "prefix" } else { "exact-path" }
            )));
        }
        Ok(())
    }

    pub(super) fn exact_path_aggregate_bridge_is_rejected() -> Result<()> {
        aggregate_bridge_without_interval_evidence_is_rejected(false)
    }

    pub(super) fn prefix_aggregate_bridge_is_rejected() -> Result<()> {
        aggregate_bridge_without_interval_evidence_is_rejected(true)
    }

    pub(super) fn authenticated_prefix_interval_preserves_later_suffix() -> Result<()> {
        let fixture = Fixture::new(0x84)?;
        let prefix = "authenticated-prefix";
        let target = fixture.target("prefix-interval", fixture.expected.baseline_root.clone());
        let evidence = IntentEvidence {
            exact_paths: Vec::new(),
            complete_prefixes: vec![DirtyPrefix {
                path: LedgerPath::parse(prefix)?,
                complete: true,
                reason: "provider_complete".into(),
                first_sequence: 1,
                last_sequence: 1,
            }],
        };
        let ledger = fixture.db.changed_path_ledger();
        let intent = prepare_intent(
            &ledger,
            &fixture.expected,
            IntentProducer::Checkout,
            &target,
            &evidence,
        )?;
        let owner_token = [fixture.expected.scope_id.0[0].wrapping_add(0x31); 32];
        let provider_id: String = fixture.db.conn.query_row(
            "SELECT provider_id FROM changed_path_scopes WHERE scope_id=?1",
            [fixture.expected.scope_id.to_text()],
            |row| row.get(0),
        )?;
        let mut writer = super::super::SegmentWriter::acquire(
            &fixture.db.db_dir.join(crate::db::DB_RELATIVE_PATH),
            &fixture
                .db
                .db_dir
                .join("observer-segments")
                .join(fixture.expected.scope_id.to_text()),
            fixture.expected.scope_id,
            fixture.expected.epoch,
            owner_token,
            &provider_id,
            b"cursor-1".to_vec(),
            Duration::from_secs(600),
        )?;
        let complete_flags =
            EvidenceFlags(EvidenceFlags::CONTENT.0 | EvidenceFlags::PROVIDER_COMPLETE_PREFIX.0);
        writer.append(&[super::super::ObserverRecord {
            sequence: 1,
            source: EvidenceSource::Observer,
            path: LedgerPath::parse(prefix)?,
            flags: complete_flags,
            provider_cursor: b"cursor-2".to_vec(),
        }])?;
        let proof_cut = writer.flush_durable()?;
        writer.rotate()?;
        fixture.insert_observer_prefix(prefix, 1)?;
        fixture.db.conn.execute(
            "UPDATE changed_path_observer_segments SET folded_end_offset=durable_end_offset
             WHERE scope_id=?1 AND segment_id=?2",
            params![fixture.expected.scope_id.to_text(), proof_cut.segment_id],
        )?;
        fixture.db.conn.execute(
            "UPDATE changed_path_observer_owners
             SET provider_identity=?1,fence_nonce=?2 WHERE scope_id=?3",
            params![
                hex::encode(&fixture.expected.provider_identity),
                b"qualified-fence".as_slice(),
                fixture.expected.scope_id.to_text(),
            ],
        )?;
        fixture.db.conn.execute(
            "UPDATE changed_path_scopes SET provider_cursor=?1,durable_offset=?2,folded_offset=?2
             WHERE scope_id=?3",
            params![
                proof_cut.provider_cursor,
                sql_u64(proof_cut.durable_end_offset, "prefix proof durable offset")?,
                fixture.expected.scope_id.to_text(),
            ],
        )?;
        let proof = fixture.proof_for_durable_segment(&proof_cut, b"cursor-1".to_vec(), 1, 0, 1)?;
        mark_filesystem_applied(&ledger, &fixture.expected, &intent, &proof)?;

        writer.append(&[super::super::ObserverRecord {
            sequence: 2,
            source: EvidenceSource::Observer,
            path: LedgerPath::parse(prefix)?,
            flags: complete_flags,
            provider_cursor: b"cursor-3".to_vec(),
        }])?;
        let suffix = writer.flush_durable()?;
        writer.rotate()?;
        drop(writer);
        fixture.insert_observer_prefix(prefix, 2)?;
        fixture.db.conn.execute(
            "UPDATE changed_path_observer_segments SET folded_end_offset=durable_end_offset
             WHERE scope_id=?1 AND segment_id=?2",
            params![fixture.expected.scope_id.to_text(), suffix.segment_id],
        )?;
        fixture.db.conn.execute(
            "UPDATE changed_path_scopes SET provider_cursor=?1,durable_offset=?2,folded_offset=?2
             WHERE scope_id=?3",
            params![
                suffix.provider_cursor,
                sql_u64(
                    suffix.durable_end_offset.max(proof_cut.durable_end_offset),
                    "prefix suffix durable offset"
                )?,
                fixture.expected.scope_id.to_text(),
            ],
        )?;
        fixture.advance_ref_to(&target)?;
        publish_intent(&ledger, &fixture.expected, &intent)?;
        let decisions = recover_scope(&ledger, &fixture.expected)?;
        if decisions != vec![(intent, RecoveryDecision::FinishPublication)] {
            return Err(Error::Corrupt(format!(
                "valid authenticated prefix interval did not publish: {decisions:?}"
            )));
        }
        let retained: (i64, i64, Option<String>) = fixture.db.conn.query_row(
            "SELECT source_mask,provider_sequence,intent_id FROM changed_path_prefixes
             WHERE scope_id=?1 AND normalized_prefix=?2",
            params![fixture.expected.scope_id.to_text(), prefix],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if retained != (EvidenceSource::Observer.mask(), 2, None) {
            return Err(Error::Corrupt(format!(
                "later authenticated prefix suffix was not retained: {retained:?}"
            )));
        }
        Ok(())
    }

    #[cfg(unix)]
    fn install_scope_directory_symlink_substitution(fixture: &Fixture) {
        use std::os::unix::fs::symlink;

        let scope_directory = fixture
            .db
            .db_dir
            .join("observer-segments")
            .join(fixture.expected.scope_id.to_text());
        let retained_directory = fixture
            .db
            .db_dir
            .join("observer-segments")
            .join(format!("{}.retained", fixture.expected.scope_id.to_text()));
        install_sidecar_ancestor_substitution_hook(move || {
            std::fs::rename(&scope_directory, &retained_directory).unwrap();
            symlink(&retained_directory, &scope_directory).unwrap();
        });
    }

    #[cfg(unix)]
    pub(super) fn ancestor_substitution_at_mark_is_rejected() -> Result<()> {
        let fixture = Fixture::new(0x85)?;
        let target = fixture.target(
            "mark-directory-swap",
            fixture.expected.baseline_root.clone(),
        );
        let ledger = fixture.db.changed_path_ledger();
        let intent = prepare_intent(
            &ledger,
            &fixture.expected,
            IntentProducer::Checkout,
            &target,
            &Fixture::evidence("mark-directory-swap.rs"),
        )?;
        let proof = fixture.qualified_proof(1, "mark-directory-swap.rs")?;
        install_scope_directory_symlink_substitution(&fixture);

        if mark_filesystem_applied(&ledger, &fixture.expected, &intent, &proof).is_ok() {
            return Err(Error::Corrupt(
                "ancestor directory substitution authorized filesystem-applied marking".into(),
            ));
        }
        Ok(())
    }

    #[cfg(unix)]
    pub(super) fn ancestor_substitution_at_recovery_is_rejected() -> Result<()> {
        let fixture = Fixture::new(0x86)?;
        let target = fixture.target(
            "recovery-directory-swap",
            fixture.expected.baseline_root.clone(),
        );
        let ledger = fixture.db.changed_path_ledger();
        let intent = prepare_intent(
            &ledger,
            &fixture.expected,
            IntentProducer::Checkout,
            &target,
            &Fixture::evidence("recovery-directory-swap.rs"),
        )?;
        let proof = fixture.qualified_proof(1, "recovery-directory-swap.rs")?;
        mark_filesystem_applied(&ledger, &fixture.expected, &intent, &proof)?;
        fixture.advance_ref_to(&target)?;
        install_scope_directory_symlink_substitution(&fixture);

        let decisions = recover_scope(&ledger, &fixture.expected)?;
        if decisions == vec![(intent, RecoveryDecision::FinishPublication)] {
            return Err(Error::Corrupt(
                "ancestor directory substitution authorized publication recovery".into(),
            ));
        }
        let trust: String = fixture.db.conn.query_row(
            "SELECT trust_state FROM changed_path_scopes WHERE scope_id=?1",
            [fixture.expected.scope_id.to_text()],
            |row| row.get(0),
        )?;
        if trust != "untrusted_gap" {
            return Err(Error::Corrupt(format!(
                "ancestor substitution did not fail closed: {trust}"
            )));
        }
        Ok(())
    }

    pub(super) fn gc_root_lifecycle() -> Result<()> {
        let mut fixture = Fixture::new(0x62)?;
        let baseline_ref = fixture.db.get_ref(&fixture.expected.ref_name)?;
        std::fs::write(
            fixture.db.workspace_root.join("intent-only.txt"),
            b"reachable only through the intent graph\n",
        )?;
        fixture.db.record(
            None,
            Some("build intent-only graph".into()),
            Actor::system(),
            false,
        )?;
        let target_ref = fixture.db.get_ref(&fixture.expected.ref_name)?;
        let files = fixture.db.load_root_files(&target_ref.root_id)?;
        let mut dependent_objects = vec![
            target_ref.root_id.0.clone(),
            target_ref.operation_id.0.clone(),
        ];
        for entry in files.values() {
            dependent_objects.push(match &entry.content {
                FileContentRef::Text(id)
                | FileContentRef::Opaque(id)
                | FileContentRef::Binary(id) => id.0.clone(),
            });
        }
        fixture.db.conn.execute(
            "UPDATE refs SET change_id=?1,root_id=?2,operation_id=?3,generation=?4,updated_at=?5
             WHERE name=?6",
            params![
                baseline_ref.change_id.0,
                baseline_ref.root_id.0,
                baseline_ref.operation_id.0,
                baseline_ref.generation,
                now_ts(),
                baseline_ref.name,
            ],
        )?;
        let target = IntentTarget {
            change_id: target_ref.change_id,
            root_id: target_ref.root_id,
            operation_id: Some(target_ref.operation_id),
        };
        prepare_intent(
            &fixture.db.changed_path_ledger(),
            &fixture.expected,
            IntentProducer::Checkout,
            &target,
            &Fixture::evidence("new.rs"),
        )?;
        fixture.db.gc(true)?;
        fixture.db.gc(false)?;
        for object_id in &dependent_objects {
            let retained: bool = fixture.db.conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM objects WHERE object_id=?1)",
                [object_id],
                |row| row.get(0),
            )?;
            if !retained {
                return Err(Error::Corrupt(format!(
                    "GC collected transitive intent dependency `{object_id}`"
                )));
            }
        }
        fixture.db.gc(false)?;
        let after_second: bool = fixture.db.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM objects WHERE object_id=?1)",
            [&target.root_id.0],
            |row| row.get(0),
        )?;
        if after_second {
            return Err(Error::Corrupt(
                "terminal intent root remained pinned".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn crash_matrix() -> Result<()> {
        let prepared = Fixture::new(0x63)?;
        let target = prepared.target("prepared", prepared.expected.baseline_root.clone());
        let intent = prepare_intent(
            &prepared.db.changed_path_ledger(),
            &prepared.expected,
            IntentProducer::LaneSync,
            &target,
            &Fixture::evidence("prepared.bin"),
        )?;
        prepared.advance_ref_to(&target)?;
        let before: i64 = prepared.db.conn.query_row(
            "SELECT continuity_generation FROM changed_path_scopes WHERE scope_id=?1",
            [prepared.expected.scope_id.to_text()],
            |row| row.get(0),
        )?;
        recover_scope(&prepared.db.changed_path_ledger(), &prepared.expected)?;
        recover_scope(&prepared.db.changed_path_ledger(), &prepared.expected)?;
        let state = prepared.db.conn.query_row(
            "SELECT lifecycle_state,trust_state,continuity_generation,s.ref_generation,
                    s.change_id,i.expected_change_id FROM changed_path_intents i
             JOIN changed_path_scopes s ON s.scope_id=i.scope_id WHERE i.intent_id=?1",
            [&intent.0],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )?;
        if state.0 != "aborted"
            || state.1 != "untrusted_gap"
            || state.2 != before + 1
            || state.3 != sql_u64(prepared.expected.ref_generation, "fixture generation")?
            || state.4 != state.5
        {
            return Err(Error::Corrupt(format!(
                "prepared crash recovery was not once-only: {state:?}"
            )));
        }

        let retired = Fixture::new(0x67)?;
        let retirement_before: i64 = retired.db.conn.query_row(
            "SELECT continuity_generation FROM changed_path_scopes WHERE scope_id=?1",
            [retired.expected.scope_id.to_text()],
            |row| row.get(0),
        )?;
        let retired_leaf = create_real_retirement_segment(&retired, 0x67)?;
        let retired_paths =
            retire_scope(&retired.db.conn, &retired.db.sqlite_path, &retired.expected)?;
        let retirement_state = retired.db.conn.query_row(
            "SELECT s.continuity_generation,s.retired_at,o.lease_state,g.state
             FROM changed_path_scopes s
             JOIN changed_path_observer_owners o ON o.scope_id=s.scope_id
             JOIN changed_path_observer_segments g ON g.scope_id=s.scope_id
             WHERE s.scope_id=?1",
            [retired.expected.scope_id.to_text()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, Option<i64>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )?;
        if retired_paths
            .iter()
            .map(|token| token.identity.original_leaf.as_str())
            .collect::<Vec<_>>()
            != vec![retired_leaf.as_str()]
            || retirement_state.0 != retirement_before + 1
            || retirement_state.1.is_none()
            || retirement_state.2 != "revoked"
            || retirement_state.3 != "retired"
        {
            return Err(Error::Corrupt(format!(
                "scope retirement was not ordered and once-only: {retirement_state:?} {retired_paths:?}"
            )));
        }

        for (tag, publish_before_recovery) in [(0x64, false), (0x65, true)] {
            let fixture = Fixture::new(tag)?;
            let target = fixture.target("published", fixture.expected.baseline_root.clone());
            let ledger = fixture.db.changed_path_ledger();
            let intent = prepare_intent(
                &ledger,
                &fixture.expected,
                IntentProducer::Materialize,
                &target,
                &Fixture::evidence("target.dat"),
            )?;
            mark_filesystem_applied(
                &ledger,
                &fixture.expected,
                &intent,
                &fixture.qualified_proof(9, "target.dat")?,
            )?;
            fixture.advance_ref_to(&target)?;
            if publish_before_recovery {
                publish_intent(&ledger, &fixture.expected, &intent)?;
            }
            recover_scope(&ledger, &fixture.expected)?;
            let state: String = fixture.db.conn.query_row(
                "SELECT lifecycle_state FROM changed_path_intents WHERE intent_id=?1",
                [&intent.0],
                |row| row.get(0),
            )?;
            if state != "acknowledged" {
                return Err(Error::Corrupt(format!(
                    "recoverable publication stayed {state}"
                )));
            }
        }
        Ok(())
    }

    pub(super) fn retirement_path_and_reader_barrier() -> Result<()> {
        let malicious = Fixture::new(0x6b)?;
        malicious.db.conn.execute(
            "INSERT INTO changed_path_observer_segments(
                 scope_id,epoch,segment_id,log_format_version,owner_token,provider_id,
                 first_sequence,last_sequence,durable_end_offset,folded_end_offset,
                 previous_segment_id,previous_segment_hash,segment_hash,segment_path,state,
                 created_at,sealed_at,updated_at)
             VALUES(?1,1,'malicious',1,'owner','provider',1,1,1,1,NULL,NULL,?2,
                    '../outside.cpl','sealed',?3,?3,?3)",
            params![
                malicious.expected.scope_id.to_text(),
                hex::encode([1_u8; 32]),
                now_ts()
            ],
        )?;
        if retire_scope(
            &malicious.db.conn,
            &malicious.db.sqlite_path,
            &malicious.expected,
        )
        .is_ok()
        {
            return Err(Error::Corrupt(
                "scope retirement returned an escaping observer segment path".into(),
            ));
        }
        let malicious_retired: bool = malicious.db.conn.query_row(
            "SELECT retired_at IS NOT NULL FROM changed_path_scopes WHERE scope_id=?1",
            [malicious.expected.scope_id.to_text()],
            |row| row.get(0),
        )?;
        if malicious_retired {
            return Err(Error::Corrupt(
                "invalid retirement path committed partial retirement".into(),
            ));
        }

        let reader_fixture = Fixture::new(0x6c)?;
        let reader_leaf = create_real_retirement_segment(&reader_fixture, 0x6c)?;
        let reader = Connection::open(reader_fixture.db.db_dir.join(crate::db::DB_RELATIVE_PATH))?;
        reader.execute_batch("BEGIN DEFERRED")?;
        let _: i64 = reader.query_row("SELECT COUNT(*) FROM changed_path_scopes", [], |row| {
            row.get(0)
        })?;
        if retire_scope(
            &reader_fixture.db.conn,
            &reader_fixture.db.sqlite_path,
            &reader_fixture.expected,
        )
        .is_ok()
        {
            return Err(Error::Corrupt(
                "retirement returned deletion authority while an older reader was active".into(),
            ));
        }
        reader.execute_batch("COMMIT")?;
        let paths = retire_scope(
            &reader_fixture.db.conn,
            &reader_fixture.db.sqlite_path,
            &reader_fixture.expected,
        )?;
        if paths
            .iter()
            .map(|token| token.identity.original_leaf.as_str())
            .collect::<Vec<_>>()
            != vec![reader_leaf.as_str()]
        {
            return Err(Error::Corrupt(format!(
                "retirement retry did not return confined paths: {paths:?}"
            )));
        }
        Ok(())
    }

    fn retired_segment_fixture(
        tag: u8,
    ) -> Result<(
        Fixture,
        Vec<SegmentDeletionToken>,
        std::path::PathBuf,
        String,
    )> {
        let fixture = Fixture::new(tag)?;
        let leaf = create_real_retirement_segment(&fixture, tag)?;
        let directory = fixture
            .db
            .db_dir
            .join("observer-segments")
            .join(fixture.expected.scope_id.to_text());
        let tokens = retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &fixture.expected)?;
        Ok((fixture, tokens, directory, leaf))
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    fn direct_target_collision_is_retained(tag: u8) -> Result<()> {
        let fixture = Fixture::new(tag)?;
        create_real_retirement_segment(&fixture, tag)?;
        let retirement =
            load_retirement_identity(&fixture.db.conn, &fixture.expected.scope_id.to_text())?
                .ok_or_else(|| Error::Corrupt("missing collision retirement identity".into()))?;
        let retirement = begin_scope_retirement(&fixture.db.conn, &retirement)?;
        let (scope_identity, rows) = inspect_segments_before_allocation(
            &fixture.db.conn,
            &fixture.db.sqlite_path,
            fixture.expected.scope_id,
            fixture.expected.epoch,
        )?;
        journal_missing_quarantine_allocations(
            &fixture.db.conn,
            &retirement,
            scope_identity,
            &rows,
        )?;
        drop(rows);
        let quarantine_leaf: String = fixture.db.conn.query_row(
            "SELECT quarantine_leaf FROM changed_path_segment_quarantine_allocations
             WHERE scope_id=?1 AND state='allocating'",
            [fixture.expected.scope_id.to_text()],
            |row| row.get(0),
        )?;
        let directory = fixture
            .db
            .db_dir
            .join("observer-segments")
            .join(fixture.expected.scope_id.to_text());
        let quarantine = directory.join(&quarantine_leaf);
        std::fs::write(&quarantine, b"foreign direct quarantine\n")?;
        let tokens = retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &fixture.expected)?;
        let (deletion_rows, audited): (i64, i64) = fixture.db.conn.query_row(
            "SELECT
                 (SELECT COUNT(*) FROM changed_path_segment_deletions WHERE scope_id=?1),
                 (SELECT COUNT(*) FROM changed_path_segment_quarantine_allocations
                  WHERE scope_id=?1 AND quarantine_leaf=?2
                    AND state='abandoned'
                    AND retained_reason='direct_quarantine_target_identity_mismatch')",
            params![fixture.expected.scope_id.to_text(), quarantine_leaf],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        if std::fs::read(&quarantine)? != b"foreign direct quarantine\n"
            || tokens.len() != 1
            || tokens[0].identity.quarantine_leaf == quarantine_leaf
            || deletion_rows != 1
            || audited != 1
        {
            return Err(Error::Corrupt(
                "direct target collision was removed or adopted".into(),
            ));
        }
        remove_retired_segments(&fixture.db.conn, &tokens)?;
        Ok(())
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn orphan_quarantine_substitution_fails_closed() -> Result<()> {
        direct_target_collision_is_retained(0x95)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn empty_orphan_quarantine_is_retained_and_rejected() -> Result<()> {
        direct_target_collision_is_retained(0x96)
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    pub(super) fn no_orphan_allocates_fresh_quarantine_authority() -> Result<()> {
        let fixture = Fixture::new(0x97)?;
        create_real_retirement_segment(&fixture, 0x97)?;
        let tokens = retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &fixture.expected)?;
        if tokens.len() != 1 || tokens[0].identity.state != "quiesced" {
            return Err(Error::Corrupt(
                "normal retirement did not mint exactly one fresh prepared authority".into(),
            ));
        }
        let quarantined = tokens[0]
            .directory
            .open_regular(&tokens[0].identity.quarantine_leaf)?;
        if super::super::secure_fs::file_identity(&quarantined)?
            != tokens[0].identity.quarantine_identity
        {
            return Err(Error::Corrupt("direct quarantine identity changed".into()));
        }
        remove_retired_segments(&fixture.db.conn, &tokens)?;
        let retry = retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &fixture.expected)?;
        remove_retired_segments(&fixture.db.conn, &retry)?;
        Ok(())
    }

    fn create_real_retirement_segment(fixture: &Fixture, tag: u8) -> Result<String> {
        create_real_segment(
            &fixture.db,
            fixture.expected.scope_id,
            fixture.expected.epoch,
            tag,
        )
    }

    fn create_real_segment(db: &Trail, scope_id: ScopeId, epoch: u64, tag: u8) -> Result<String> {
        let (leaf, writer) = create_retained_real_segment(db, scope_id, epoch, tag)?;
        drop(writer);
        Ok(leaf)
    }

    fn create_retained_real_segment(
        db: &Trail,
        scope_id: ScopeId,
        epoch: u64,
        tag: u8,
    ) -> Result<(String, super::super::SegmentWriter)> {
        let directory = db.db_dir.join("observer-segments").join(scope_id.to_text());
        let provider_id: String = db.conn.query_row(
            "SELECT provider_id FROM changed_path_scopes WHERE scope_id=?1",
            [scope_id.to_text()],
            |row| row.get(0),
        )?;
        let writer = super::super::SegmentWriter::acquire(
            &db.sqlite_path,
            &directory,
            scope_id,
            epoch,
            [tag.wrapping_add(0x41); 32],
            &provider_id,
            b"retirement-test-cursor".to_vec(),
            Duration::from_secs(600),
        )?;
        let leaf: String = db.conn.query_row(
            "SELECT segment_path FROM changed_path_observer_segments
             WHERE scope_id=?1 AND epoch=?2",
            params![scope_id.to_text(), sql_u64(epoch, "scope epoch")?],
            |row| row.get(0),
        )?;
        Ok((leaf, writer))
    }

    pub(super) fn retirement_requires_retained_writer_quiescence() -> Result<()> {
        let fixture = Fixture::new(0xa1)?;
        let (leaf, mut writer) = create_retained_real_segment(
            &fixture.db,
            fixture.expected.scope_id,
            fixture.expected.epoch,
            0xa1,
        )?;
        let original = fixture
            .db
            .db_dir
            .join("observer-segments")
            .join(fixture.expected.scope_id.to_text())
            .join(&leaf);
        if retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &fixture.expected).is_ok() {
            return Err(Error::Corrupt(
                "retirement moved a segment while its writer FD remained retained".into(),
            ));
        }
        if !original.exists() {
            return Err(Error::Corrupt(
                "retirement moved the segment before writer close acknowledgement".into(),
            ));
        }
        let quiesced: i64 = fixture.db.conn.query_row(
            "SELECT COUNT(*) FROM changed_path_segment_deletions WHERE scope_id=?1",
            [fixture.expected.scope_id.to_text()],
            |row| row.get(0),
        )?;
        if quiesced != 0 {
            return Err(Error::Corrupt(
                "retirement published quiesced deletion before writer close acknowledgement".into(),
            ));
        }
        let before_append = std::fs::metadata(&original)?.len();
        let late_record = super::super::ObserverRecord {
            sequence: 1,
            source: EvidenceSource::Observer,
            path: LedgerPath::parse("late-after-retirement-fence.txt")?,
            flags: EvidenceFlags::CONTENT,
            provider_cursor: b"late-after-retirement-fence".to_vec(),
        };
        if writer.append(&[late_record]).is_ok()
            || std::fs::metadata(&original)?.len() != before_append
        {
            return Err(Error::Corrupt(
                "revoked retained writer appended after the retirement fence".into(),
            ));
        }
        drop(writer);
        let retry = retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &fixture.expected)?;
        if retry.len() != 1 || original.exists() {
            return Err(Error::Corrupt(
                "retirement did not converge after the retained writer closed".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn deletion_parent_substitution_uses_retained_authority() -> Result<()> {
        let (fixture, tokens, directory, leaf) = retired_segment_fixture(0x87)?;
        let retained = directory.with_extension("retained-directory");
        let replacement_file = directory.join(&leaf);
        let original_file = retained.join(&leaf);
        let hook_directory = directory.clone();
        let hook_retained = retained.clone();
        let hook_leaf = leaf.clone();
        install_deletion_substitution_hook(DeletionSubstitutionPoint::Parent, move || {
            std::fs::rename(&hook_directory, &hook_retained).unwrap();
            std::fs::create_dir(&hook_directory).unwrap();
            std::fs::write(
                hook_directory.join(&hook_leaf),
                b"replacement must survive\n",
            )
            .unwrap();
        });

        remove_retired_segments(&fixture.db.conn, &tokens)?;
        remove_retired_segments(&fixture.db.conn, &tokens)?;
        if original_file.exists() || !replacement_file.exists() {
            return Err(Error::Corrupt(
                "retired deletion followed a substituted parent directory".into(),
            ));
        }
        Ok(())
    }

    #[cfg(unix)]
    pub(super) fn deletion_leaf_substitution_fails_closed() -> Result<()> {
        use std::os::unix::fs::symlink;

        let (fixture, tokens, directory, segment_leaf) = retired_segment_fixture(0x88)?;
        let leaf = directory.join(segment_leaf);
        let outside = fixture._root.path().join("outside-segment.cpl");
        std::fs::write(&outside, b"outside must survive\n")?;
        symlink(&outside, &leaf)?;

        if remove_retired_segments(&fixture.db.conn, &tokens).is_ok() {
            return Err(Error::Corrupt(
                "retired deletion accepted a substituted leaf".into(),
            ));
        }
        if !outside.exists() || std::fs::symlink_metadata(&leaf).is_err() {
            return Err(Error::Corrupt(
                "retired deletion escaped its descriptor-relative leaf".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn deletion_name_substitution_after_verification_fails_closed() -> Result<()> {
        let (fixture, tokens, directory, segment_leaf) = retired_segment_fixture(0x89)?;
        let leaf = directory.join(segment_leaf);
        let authorized = directory.join("post-verification-authorized.cpl");
        let quarantine = directory.join(&tokens[0].identity.quarantine_leaf);
        std::fs::write(&leaf, b"replacement must survive\n")?;

        if remove_retired_segments(&fixture.db.conn, &tokens).is_ok() {
            return Err(Error::Corrupt(
                "retired deletion accepted a name substituted after verification".into(),
            ));
        }
        if !leaf.exists() || authorized.exists() || !quarantine.exists() {
            return Err(Error::Corrupt(
                "retired deletion did not preserve the authorized inode and quarantine the substitution"
                    .into(),
            ));
        }
        Ok(())
    }

    pub(super) fn deletion_substitution_after_quarantine_verification_fails_closed() -> Result<()> {
        let (fixture, tokens, directory, _segment_leaf) = retired_segment_fixture(0x8c)?;
        let quarantine = directory.join(&tokens[0].identity.quarantine_leaf);
        let authorized = directory.join("post-quarantine-authorized.cpl");
        std::fs::rename(&quarantine, &authorized)?;
        std::fs::write(&quarantine, b"replacement must survive\n")?;

        let rejection = remove_retired_segments(&fixture.db.conn, &tokens)
            .err()
            .ok_or_else(|| {
                Error::Corrupt(
                    "retired deletion accepted substitution after quarantine verification".into(),
                )
            })?;
        if !quarantine.exists() || !authorized.exists() {
            return Err(Error::Corrupt(format!(
                "retired deletion did not preserve post-verification files: quarantine={} authorized={} directory={} rejection={rejection}",
                quarantine.exists(),
                authorized.exists(),
                directory.display()
            )));
        }
        Ok(())
    }

    pub(super) fn deletion_retry_rejects_hostile_quarantine_replacement() -> Result<()> {
        let (fixture, tokens, directory, _segment_leaf) = retired_segment_fixture(0x8d)?;
        let quarantine = directory.join(&tokens[0].identity.quarantine_leaf);
        let retained = directory.join("hostile-quarantine-retained.cpl");
        std::fs::rename(&quarantine, &retained)?;
        std::fs::write(&quarantine, b"hostile replacement must survive\n")?;

        if retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &fixture.expected).is_ok() {
            return Err(Error::Corrupt(
                "retirement retry adopted a hostile quarantine replacement".into(),
            ));
        }
        if !quarantine.exists() || !retained.exists() {
            return Err(Error::Corrupt(
                "retirement retry deleted a hostile or retained quarantine file".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn deletion_normal_retry_is_durably_idempotent() -> Result<()> {
        let (fixture, tokens, directory, segment_leaf) = retired_segment_fixture(0x8e)?;
        let original = directory.join(&segment_leaf);
        let quarantine = directory.join(&tokens[0].identity.quarantine_leaf);
        let expected_bytes = std::fs::read(&quarantine)?;
        let expected_identity = tokens[0].identity.segment_identity;
        remove_retired_segments(&fixture.db.conn, &tokens)?;
        if original.exists() || !quarantine.exists() {
            return Err(Error::Corrupt(
                "normal retirement did not retain exactly one quarantined segment".into(),
            ));
        }
        let retained = std::fs::File::open(&quarantine)?;
        if super::super::secure_fs::file_identity(&retained)? != expected_identity
            || std::fs::read(&quarantine)? != expected_bytes
        {
            return Err(Error::Corrupt(
                "quiesced retirement did not retain the authenticated segment".into(),
            ));
        }
        let retry = retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &fixture.expected)?;
        remove_retired_segments(&fixture.db.conn, &retry)?;
        remove_retired_segments(&fixture.db.conn, &retry)?;
        let reopened = Connection::open(&fixture.db.sqlite_path)?;
        let reopened_retry = retire_scope(&reopened, &fixture.db.sqlite_path, &fixture.expected)?;
        remove_retired_segments(&reopened, &reopened_retry)?;
        let quiesced: i64 = reopened.query_row(
            "SELECT COUNT(*) FROM changed_path_segment_deletions
             WHERE scope_id=?1 AND epoch=?2 AND state='quiesced' AND completed_at IS NOT NULL",
            params![
                fixture.expected.scope_id.to_text(),
                sql_u64(fixture.expected.epoch, "scope epoch")?
            ],
            |row| row.get(0),
        )?;
        if quiesced != 1 || !quarantine.exists() || original.exists() {
            return Err(Error::Corrupt(format!(
                "normal deletion retry did not retain one durable quiesced row and segment: rows={quiesced} quarantine={} original={}",
                quarantine.exists(),
                original.exists()
            )));
        }
        Ok(())
    }

    pub(super) fn deletion_quiesced_retry_rejects_missing_quarantine() -> Result<()> {
        let (fixture, tokens, directory, segment_leaf) = retired_segment_fixture(0x8f)?;
        let original = directory.join(segment_leaf);
        let quarantine = directory.join(&tokens[0].identity.quarantine_leaf);
        let retained = directory.join("missing-quarantine-retained.cpl");
        remove_retired_segments(&fixture.db.conn, &tokens)?;
        std::fs::rename(&quarantine, &retained)?;

        if retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &fixture.expected).is_ok() {
            return Err(Error::Corrupt(
                "quiesced retirement retry accepted a missing quarantine entry".into(),
            ));
        }
        if !retained.exists() || quarantine.exists() || original.exists() {
            return Err(Error::Corrupt(
                "missing-quarantine retry mutated retained filesystem evidence".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn deletion_quiesced_retry_rejects_reappeared_original() -> Result<()> {
        let (fixture, tokens, directory, segment_leaf) = retired_segment_fixture(0x94)?;
        let original = directory.join(segment_leaf);
        let quarantine = directory.join(&tokens[0].identity.quarantine_leaf);
        remove_retired_segments(&fixture.db.conn, &tokens)?;
        std::fs::write(&original, b"reappeared original-name replacement\n")?;

        let retry = retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &fixture.expected)?;
        if remove_retired_segments(&fixture.db.conn, &retry).is_ok() {
            return Err(Error::Corrupt(
                "quiesced retirement retry accepted a reappeared original name".into(),
            ));
        }
        if !original.exists() || !quarantine.exists() {
            return Err(Error::Corrupt(
                "reappeared-original retry removed filesystem evidence".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn restored_nullable_provider_lane_deletion() -> Result<()> {
        let root = tempfile::tempdir()?;
        std::fs::write(root.path().join("lane.txt"), b"lane\n")?;
        Trail::init(root.path(), "main", InitImportMode::WorkingTree, false)?;
        let mut db = Trail::open(root.path())?;
        let spawned = db.spawn_lane("restored-retire", Some("main"), true, None, None)?;
        let branch = db.lane_branch("restored-retire")?;
        let head = db.get_ref(&branch.ref_name)?;
        let view = db.create_workspace_view(
            &branch.lane_id,
            &head.change_id,
            &head.root_id,
            "fuse-cow",
            &root.path().join("restored-retire-view"),
        )?;
        let baseline = BaselineIdentity {
            ref_name: head.name.clone(),
            ref_generation: u64::try_from(head.generation)
                .map_err(|_| Error::Corrupt("negative lane ref generation".into()))?,
            change_id: head.change_id.clone(),
            root_id: head.root_id.clone(),
        };
        let policy = PolicyIdentity {
            fingerprint: [0x8a; 32],
            generation: 1,
        };
        let filesystem = FilesystemIdentity(vec![0x8a]);
        let provider = ProviderIdentity {
            identity: vec![0x8b],
            capabilities: ProviderCapabilities {
                durable_cursor: true,
                linearizable_fence: true,
                rename_pairing: true,
                overflow_scope: true,
                filesystem_supported: true,
                clean_proof_allowed: true,
                power_loss_durability: true,
            },
        };
        for identity in [
            ScopeIdentity {
                scope_id: ScopeId([0x8a; 32]),
                kind: ScopeKind::MaterializedLane,
                owner_id: branch.lane_id.clone(),
            },
            ScopeIdentity {
                scope_id: ScopeId([0x8b; 32]),
                kind: ScopeKind::WorkspaceView,
                owner_id: view.view_id.clone(),
            },
        ] {
            db.changed_path_ledger().begin_scope(
                &identity,
                &baseline,
                &policy,
                &filesystem,
                &provider,
            )?;
        }
        db.conn.execute(
            "UPDATE changed_path_scopes SET scope_root=?1 WHERE scope_id=?2",
            params![spawned.workdir, ScopeId([0x8a; 32]).to_text()],
        )?;
        let backup = root.path().join("restored-retire-backup");
        db.create_backup(&backup, false)?;
        let destination = root.path().join("restored-retire-destination");
        Trail::restore_backup(&destination, &backup, false)?;
        let mut restored = Trail::open(&destination)?;
        let nullable: i64 = restored.conn.query_row(
            "SELECT COUNT(*) FROM changed_path_scopes
             WHERE scope_kind IN ('materialized_lane','workspace_view')
               AND provider_identity IS NULL AND retired_at IS NULL",
            [],
            |row| row.get(0),
        )?;
        if nullable != 2 {
            return Err(Error::Corrupt(format!(
                "restore did not produce two nullable-provider deletion scopes: {nullable}"
            )));
        }
        restored.remove_lane("restored-retire", true)?;
        let retired: i64 = restored.conn.query_row(
            "SELECT COUNT(*) FROM changed_path_scopes
             WHERE scope_kind IN ('materialized_lane','workspace_view') AND retired_at IS NOT NULL",
            [],
            |row| row.get(0),
        )?;
        if retired != 2 {
            return Err(Error::Corrupt(format!(
                "restored nullable-provider lane/view scopes were not retired: {retired}"
            )));
        }
        Ok(())
    }

    #[cfg(unix)]
    pub(super) fn non_utf_database_path_mark_recover_and_retire() -> Result<()> {
        use std::os::unix::ffi::{OsStrExt, OsStringExt};

        let fixture = Fixture::new(0x8c)?;
        let Fixture {
            _root,
            db,
            expected,
            backup,
            restore,
        } = fixture;
        let old_workspace = db.workspace_root.clone();
        drop(db);
        let new_workspace = _root
            .path()
            .join(std::ffi::OsString::from_vec(b"workspace-\xff".to_vec()));
        if let Err(error) = std::fs::rename(&old_workspace, &new_workspace) {
            if cfg!(target_os = "macos") && error.raw_os_error() == Some(libc::EILSEQ) {
                let conn = Connection::open_in_memory()?;
                let database_path = new_workspace.join(".trail/index/trail.sqlite");
                let ledger = ChangedPathLedger::new_at(&conn, &database_path);
                if ledger.database_path()?.as_os_str().as_bytes()
                    != database_path.as_os_str().as_bytes()
                {
                    return Err(Error::Corrupt(
                        "macOS lossless database path plumbing changed raw bytes".into(),
                    ));
                }
                return Ok(());
            }
            return Err(error.into());
        }
        let db = Trail::open(&new_workspace)?;
        let fixture = Fixture {
            _root,
            db,
            expected,
            backup,
            restore,
        };
        let target = fixture.target(
            "non-utf-database-path",
            fixture.expected.baseline_root.clone(),
        );
        let ledger = fixture.db.changed_path_ledger();
        let intent = prepare_intent(
            &ledger,
            &fixture.expected,
            IntentProducer::Materialize,
            &target,
            &Fixture::evidence("non-utf.txt"),
        )?;
        let proof = fixture.qualified_proof(1, "non-utf.txt")?;
        mark_filesystem_applied(&ledger, &fixture.expected, &intent, &proof)?;
        fixture.advance_ref_to(&target)?;
        recover_scope(&ledger, &fixture.expected)?;

        let mut retired_expected = fixture.expected.clone();
        retired_expected.ref_generation = retired_expected.ref_generation.saturating_add(1);
        let tokens = retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &retired_expected)?;
        remove_retired_segments(&fixture.db.conn, &tokens)?;
        if tokens.is_empty()
            || tokens.iter().any(|token| {
                token
                    .directory
                    .open_regular(&token.identity.original_leaf)
                    .is_ok()
            })
        {
            return Err(Error::Corrupt(
                "non-UTF database path did not retain proof/recovery/retirement authority".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn lane_deletion_retires_scope_first() -> Result<()> {
        let root = tempfile::tempdir()?;
        std::fs::write(root.path().join("lane.txt"), b"lane\n")?;
        Trail::init(root.path(), "main", InitImportMode::WorkingTree, false)?;
        let mut db = Trail::open(root.path())?;
        let spawned = db.spawn_lane("retire-first", Some("main"), true, None, None)?;
        let branch = db.lane_branch("retire-first")?;
        let head = db.get_ref(&branch.ref_name)?;
        let view = db.create_workspace_view(
            &branch.lane_id,
            &head.change_id,
            &head.root_id,
            "fuse-cow",
            &root.path().join("retire-first-view"),
        )?;
        let scope = ScopeIdentity {
            scope_id: ScopeId([0x6d; 32]),
            kind: ScopeKind::MaterializedLane,
            owner_id: branch.lane_id.clone(),
        };
        let baseline = BaselineIdentity {
            ref_name: head.name.clone(),
            ref_generation: u64::try_from(head.generation)
                .map_err(|_| Error::Corrupt("negative lane ref generation".into()))?,
            change_id: head.change_id.clone(),
            root_id: head.root_id.clone(),
        };
        let policy = PolicyIdentity {
            fingerprint: [0x6d; 32],
            generation: 1,
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
                power_loss_durability: true,
            },
        };
        db.changed_path_ledger()
            .begin_scope(&scope, &baseline, &policy, &filesystem, &provider)?;
        let view_scope = ScopeIdentity {
            scope_id: ScopeId([0x6e; 32]),
            kind: ScopeKind::WorkspaceView,
            owner_id: view.view_id.clone(),
        };
        db.changed_path_ledger().begin_scope(
            &view_scope,
            &baseline,
            &policy,
            &filesystem,
            &provider,
        )?;
        db.conn.execute(
            "UPDATE changed_path_scopes SET scope_root=?1 WHERE scope_id=?2",
            params![spawned.workdir, scope.scope_id.to_text()],
        )?;
        db.conn.execute(
            "INSERT INTO changed_path_observer_segments(
                 scope_id,epoch,segment_id,log_format_version,owner_token,provider_id,
                 first_sequence,last_sequence,durable_end_offset,folded_end_offset,
                 previous_segment_id,previous_segment_hash,segment_hash,segment_path,state,
                 created_at,sealed_at,updated_at)
             VALUES(?1,1,'lane-segment',1,'owner','provider',1,1,1,1,NULL,NULL,?2,
                    '../escape.cpl','sealed',?3,?3,?3)",
            params![scope.scope_id.to_text(), hex::encode([3_u8; 32]), now_ts()],
        )?;
        db.conn.execute(
            "INSERT INTO changed_path_observer_segments(
                 scope_id,epoch,segment_id,log_format_version,owner_token,provider_id,
                 first_sequence,last_sequence,durable_end_offset,folded_end_offset,
                 previous_segment_id,previous_segment_hash,segment_hash,segment_path,state,
                 created_at,sealed_at,updated_at)
             VALUES(?1,1,'view-segment',1,'owner','provider',1,1,1,1,NULL,NULL,?2,
                    'view-segment.cpl','sealed',?3,?3,?3)",
            params![
                view_scope.scope_id.to_text(),
                hex::encode([4_u8; 32]),
                now_ts()
            ],
        )?;
        if db.remove_lane("retire-first", true).is_ok() {
            return Err(Error::Corrupt(
                "lane deletion bypassed changed-path scope retirement".into(),
            ));
        }
        let workdir = std::path::PathBuf::from(
            spawned
                .workdir
                .ok_or_else(|| Error::Corrupt("materialized lane has no workdir".into()))?,
        );
        if !workdir.exists() || db.try_get_ref(&branch.ref_name)?.is_none() {
            return Err(Error::Corrupt(
                "lane deletion mutated filesystem/ref before retirement completed".into(),
            ));
        }
        db.conn.execute(
            "DELETE FROM changed_path_observer_segments WHERE scope_id=?1",
            [scope.scope_id.to_text()],
        )?;
        db.conn.execute(
            "DELETE FROM changed_path_observer_segments WHERE scope_id=?1",
            [view_scope.scope_id.to_text()],
        )?;
        let lane_leaf = create_real_segment(&db, scope.scope_id, 1, 0x6d)?;
        let view_leaf = create_real_segment(&db, view_scope.scope_id, 1, 0x6e)?;
        let segment_directory = db
            .db_dir
            .join("observer-segments")
            .join(scope.scope_id.to_text());
        let segment_file = segment_directory.join(lane_leaf);
        let view_segment_directory = db
            .db_dir
            .join("observer-segments")
            .join(view_scope.scope_id.to_text());
        let view_segment_file = view_segment_directory.join(view_leaf);
        let reader = Connection::open(db.db_dir.join(crate::db::DB_RELATIVE_PATH))?;
        reader.execute_batch("BEGIN DEFERRED")?;
        let _: i64 = reader.query_row("SELECT COUNT(*) FROM changed_path_scopes", [], |row| {
            row.get(0)
        })?;
        if db.remove_lane("retire-first", true).is_ok() {
            return Err(Error::Corrupt(
                "lane deletion bypassed the pre-retirement reader barrier".into(),
            ));
        }
        if db.remove_lane("retire-first", true).is_ok() {
            return Err(Error::Corrupt(
                "lane deletion retry bypassed the still-active reader barrier".into(),
            ));
        }
        if !workdir.exists() || db.try_get_ref(&branch.ref_name)?.is_none() {
            return Err(Error::Corrupt(
                "lane deletion proceeded before the pre-retirement reader drained".into(),
            ));
        }
        reader.execute_batch("COMMIT")?;
        db.remove_lane("retire-first", true)?;
        let retired: i64 = db.conn.query_row(
            "SELECT COUNT(*) FROM changed_path_scopes
             WHERE scope_id IN (?1,?2) AND retired_at IS NOT NULL",
            params![scope.scope_id.to_text(), view_scope.scope_id.to_text()],
            |row| row.get(0),
        )?;
        if retired != 2 || workdir.exists() || segment_file.exists() || view_segment_file.exists() {
            return Err(Error::Corrupt(
                "lane/view scopes or segments were not retired before workdir deletion".into(),
            ));
        }
        Ok(())
    }

    #[test]
    fn changed_path_subprocess_crash_helper() {
        let Some(workspace) = std::env::var_os("TRAIL_TEST_CHANGED_PATH_CRASH_WORKSPACE") else {
            return;
        };
        run_real_crash_scenario(std::path::Path::new(&workspace)).unwrap();
        panic!("changed-path crash helper passed its requested crash point");
    }

    #[test]
    fn deletion_subprocess_crash_helper() {
        let Some(workspace) = std::env::var_os("TRAIL_TEST_DELETION_CRASH_WORKSPACE") else {
            return;
        };
        let lane = std::env::var("TRAIL_TEST_DELETION_CRASH_LANE").unwrap();
        let mut db = Trail::open(std::path::Path::new(&workspace)).unwrap();
        db.remove_lane(&lane, true).unwrap();
        panic!("deletion crash helper passed its requested crash point");
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn retirement_never_uses_separable_mkdir_open_authority() {
        use std::os::unix::fs::PermissionsExt;
        use std::sync::atomic::{AtomicBool, Ordering};

        let fixture = Fixture::new(0xa4).unwrap();
        create_real_retirement_segment(&fixture, 0xa4).unwrap();
        let directory = fixture
            .db
            .db_dir
            .join("observer-segments")
            .join(fixture.expected.scope_id.to_text());
        let hook_ran = std::sync::Arc::new(AtomicBool::new(false));
        let hook_ran_capture = hook_ran.clone();
        let hook_directory = directory.clone();
        super::super::secure_fs::install_private_dir_create_open_hook(move || {
            let allocated = std::fs::read_dir(&hook_directory)
                .unwrap()
                .filter_map(std::result::Result::ok)
                .find(|entry| {
                    entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with(".trail-delete-")
                        && entry
                            .path()
                            .extension()
                            .is_some_and(|extension| extension == "d")
                })
                .unwrap();
            let retained = allocated.path().with_extension("old-retained");
            std::fs::rename(allocated.path(), retained).unwrap();
            std::fs::create_dir(allocated.path()).unwrap();
            std::fs::set_permissions(allocated.path(), std::fs::Permissions::from_mode(0o700))
                .unwrap();
            hook_ran_capture.store(true, Ordering::SeqCst);
        });

        let result = retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &fixture.expected);
        super::super::secure_fs::clear_private_dir_create_open_hook();

        result.unwrap();
        assert!(
            !hook_ran.load(Ordering::SeqCst),
            "retirement crossed the separable mkdirat/openat substitution window"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn direct_quarantine_rejects_source_substitution_before_atomic_rename() {
        let fixture = Fixture::new(0xa5).unwrap();
        let source_leaf = create_real_retirement_segment(&fixture, 0xa5).unwrap();
        let directory = fixture
            .db
            .db_dir
            .join("observer-segments")
            .join(fixture.expected.scope_id.to_text());
        let source = directory.join(&source_leaf);
        let retained = directory.join("authenticated-source-retained.cpl");
        let hook_source = source.clone();
        let hook_retained = retained.clone();
        install_deletion_substitution_hook(
            DeletionSubstitutionPoint::BeforeQuarantineMove,
            move || {
                std::fs::rename(&hook_source, &hook_retained).unwrap();
                std::fs::write(&hook_source, b"hostile source replacement\n").unwrap();
            },
        );

        let result = retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &fixture.expected);
        clear_deletion_substitution_hook();
        assert!(result.is_err(), "substituted source was accepted");
        assert!(retained.is_file(), "authenticated source was not retained");
        let targets = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".trail-delete-")
            })
            .collect::<Vec<_>>();
        assert_eq!(targets.len(), 1);
        assert_eq!(
            std::fs::read(targets[0].path()).unwrap(),
            b"hostile source replacement\n"
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn direct_quarantine_rejects_target_substitution_after_rename_before_verify() {
        let fixture = Fixture::new(0xa6).unwrap();
        create_real_retirement_segment(&fixture, 0xa6).unwrap();
        let directory = fixture
            .db
            .db_dir
            .join("observer-segments")
            .join(fixture.expected.scope_id.to_text());
        let retained = directory.join("renamed-target-retained.cpl");
        let hook_directory = directory.clone();
        let hook_retained = retained.clone();
        install_deletion_substitution_hook(
            DeletionSubstitutionPoint::AfterDirectRenameBeforeVerify,
            move || {
                let target = std::fs::read_dir(&hook_directory)
                    .unwrap()
                    .filter_map(std::result::Result::ok)
                    .find(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".trail-delete-")
                    })
                    .unwrap()
                    .path();
                std::fs::rename(&target, &hook_retained).unwrap();
                std::fs::write(&target, b"hostile target replacement\n").unwrap();
            },
        );

        let result = retire_scope(&fixture.db.conn, &fixture.db.sqlite_path, &fixture.expected);
        clear_deletion_substitution_hook();
        assert!(result.is_err(), "substituted direct target was accepted");
        assert!(
            retained.is_file(),
            "atomically moved source was not retained"
        );
        let hostile = std::fs::read_dir(&directory)
            .unwrap()
            .filter_map(std::result::Result::ok)
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".trail-delete-")
            })
            .unwrap()
            .path();
        assert_eq!(
            std::fs::read(hostile).unwrap(),
            b"hostile target replacement\n"
        );
    }

    #[test]
    fn subprocess_kill_and_retry_covers_quarantine_deletion_boundaries() {
        for (index, phase) in [
            "changed_path_deletion_after_retirement_fence_barrier",
            "changed_path_deletion_after_allocation_journal_barrier",
            "changed_path_deletion_after_direct_quarantine_rename",
            "changed_path_deletion_after_direct_quarantine_verify",
            "changed_path_deletion_after_direct_quarantine_fsync",
            "changed_path_deletion_after_allocation_identity_barrier",
            "changed_path_deletion_between_allocation_segments",
            "changed_path_deletion_after_allocation_setup",
            "changed_path_deletion_before_retirement_commit",
            "changed_path_deletion_after_retirement_commit",
            "changed_path_deletion_after_retirement_wal_barrier",
        ]
        .into_iter()
        .enumerate()
        {
            let root = tempfile::tempdir().unwrap();
            std::fs::write(root.path().join("lane.txt"), b"lane\n").unwrap();
            Trail::init(root.path(), "main", InitImportMode::WorkingTree, false).unwrap();
            let mut db = Trail::open(root.path()).unwrap();
            let lane = format!("quarantine-crash-{index}");
            let spawned = db
                .spawn_lane(&lane, Some("main"), true, None, None)
                .unwrap();
            let branch = db.lane_branch(&lane).unwrap();
            let head = db.get_ref(&branch.ref_name).unwrap();
            let scope = ScopeIdentity {
                scope_id: ScopeId([0x90_u8.wrapping_add(index as u8); 32]),
                kind: ScopeKind::MaterializedLane,
                owner_id: branch.lane_id.clone(),
            };
            db.changed_path_ledger()
                .begin_scope(
                    &scope,
                    &BaselineIdentity {
                        ref_name: head.name,
                        ref_generation: u64::try_from(head.generation).unwrap(),
                        change_id: head.change_id,
                        root_id: head.root_id,
                    },
                    &PolicyIdentity {
                        fingerprint: [0x90; 32],
                        generation: 1,
                    },
                    &FilesystemIdentity(vec![0x91]),
                    &ProviderIdentity {
                        identity: vec![0x92],
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
                )
                .unwrap();
            db.conn
                .execute(
                    "UPDATE changed_path_scopes SET scope_root=?1 WHERE scope_id=?2",
                    params![spawned.workdir, scope.scope_id.to_text()],
                )
                .unwrap();
            let segment_directory = db
                .db_dir
                .join("observer-segments")
                .join(scope.scope_id.to_text());
            let provider_id: String = db
                .conn
                .query_row(
                    "SELECT provider_id FROM changed_path_scopes WHERE scope_id=?1",
                    [scope.scope_id.to_text()],
                    |row| row.get(0),
                )
                .unwrap();
            let mut writer = super::super::SegmentWriter::acquire(
                &db.sqlite_path,
                &segment_directory,
                scope.scope_id,
                1,
                [0x93_u8.wrapping_add(index as u8); 32],
                &provider_id,
                b"crash-retirement-cursor".to_vec(),
                Duration::from_secs(600),
            )
            .unwrap();
            writer
                .append(&[super::super::ObserverRecord {
                    sequence: 1,
                    source: EvidenceSource::Observer,
                    path: LedgerPath::parse("allocation-crash-segment.txt").unwrap(),
                    flags: EvidenceFlags::CONTENT,
                    provider_cursor: b"crash-retirement-cursor-2".to_vec(),
                }])
                .unwrap();
            writer.flush_durable().unwrap();
            writer.rotate().unwrap();
            let leaves = db
                .conn
                .prepare(
                    "SELECT segment_path FROM changed_path_observer_segments
                     WHERE scope_id=?1 AND epoch=1 ORDER BY first_sequence,segment_id",
                )
                .unwrap()
                .query_map([scope.scope_id.to_text()], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap();
            drop(writer);
            assert_eq!(leaves.len(), 2);
            let mut expected_segment_bytes = leaves
                .iter()
                .map(|leaf| std::fs::read(segment_directory.join(leaf)).unwrap())
                .collect::<Vec<_>>();
            expected_segment_bytes.sort();
            let workdir = std::path::PathBuf::from(spawned.workdir.unwrap());
            let sqlite_path = db.sqlite_path.clone();
            drop(db);

            let ready = root.path().join(format!("{phase}.ready"));
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "db::change_ledger::recovery::harness::deletion_subprocess_crash_helper",
                    "--nocapture",
                ])
                .env("RUST_TEST_THREADS", "1")
                .env("TRAIL_TEST_CRASH_AT", phase)
                .env("TRAIL_TEST_CRASH_READY", &ready)
                .env("TRAIL_TEST_DELETION_CRASH_WORKSPACE", root.path())
                .env("TRAIL_TEST_DELETION_CRASH_LANE", &lane)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            wait_for_crash_handshake(&mut child, &ready, phase);
            child.kill().unwrap();
            let _ = child.wait().unwrap();

            let mut injected_conflicts = Vec::new();
            if phase == "changed_path_deletion_after_allocation_journal_barrier" {
                let crash_conn = Connection::open(&sqlite_path).unwrap();
                let allocation_leaves = crash_conn
                    .prepare(
                        "SELECT quarantine_leaf
                         FROM changed_path_segment_quarantine_allocations
                         WHERE scope_id=?1 AND state='allocating' ORDER BY segment_id",
                    )
                    .unwrap()
                    .query_map([scope.scope_id.to_text()], |row| row.get::<_, String>(0))
                    .unwrap()
                    .collect::<std::result::Result<Vec<_>, _>>()
                    .unwrap();
                drop(crash_conn);
                for leaf in allocation_leaves {
                    let path = segment_directory.join(leaf);
                    std::fs::write(&path, b"foreign direct crash target\n").unwrap();
                    injected_conflicts.push(path);
                }
            }

            let mut reopened = Trail::open(root.path()).unwrap();
            reopened.remove_lane(&lane, true).unwrap();
            let retired: bool = reopened
                .conn
                .query_row(
                    "SELECT retired_at IS NOT NULL FROM changed_path_scopes WHERE scope_id=?1",
                    [scope.scope_id.to_text()],
                    |row| row.get(0),
                )
                .unwrap();
            let retained_quarantines = reopened
                .conn
                .prepare(
                    "SELECT quarantine_leaf FROM changed_path_segment_deletions
                     WHERE scope_id=?1 AND epoch=1 ORDER BY segment_id",
                )
                .unwrap()
                .query_map([scope.scope_id.to_text()], |row| row.get::<_, String>(0))
                .unwrap()
                .map(|leaf| segment_directory.join(leaf.unwrap()))
                .collect::<Vec<_>>();
            assert!(retired, "scope did not remain retired after {phase}");
            for leaf in &leaves {
                assert!(
                    !segment_directory.join(leaf).exists(),
                    "original segment name remained after {phase}: {leaf}"
                );
            }
            assert_eq!(
                retained_quarantines.len(),
                2,
                "exactly two quarantined segments were not retained after {phase}"
            );
            let mut retained_bytes = retained_quarantines
                .iter()
                .map(|path| std::fs::read(path).unwrap())
                .collect::<Vec<_>>();
            retained_bytes.sort();
            assert_eq!(retained_bytes, expected_segment_bytes);
            for conflict in &injected_conflicts {
                assert!(
                    conflict.is_file(),
                    "foreign namespace was removed after {phase}"
                );
            }
            assert!(!workdir.exists(), "workdir remained after {phase}");
            let quiesced: i64 = reopened
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM changed_path_segment_deletions
                     WHERE scope_id=?1 AND epoch=1 AND state='quiesced'
                       AND completed_at IS NOT NULL",
                    [scope.scope_id.to_text()],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                quiesced, 2,
                "retirement was not durably quiesced after {phase}"
            );
            let (bound, abandoned): (i64, i64) = reopened
                .conn
                .query_row(
                    "SELECT
                         SUM(state='bound'),SUM(state='abandoned')
                     FROM changed_path_segment_quarantine_allocations WHERE scope_id=?1",
                    [scope.scope_id.to_text()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            assert_eq!(bound, 2, "allocations were not exactly bound after {phase}");
            if phase == "changed_path_deletion_after_allocation_journal_barrier" {
                assert_eq!(abandoned, 2, "foreign allocations were not audited");
            }
        }
    }

    #[test]
    fn subprocess_kill_and_reopen_covers_intent_durability_boundaries() {
        for (index, (phase, tamper)) in [
            ("changed_path_after_object_graph", None),
            ("changed_path_after_intent_prepare", None),
            ("changed_path_after_filesystem_write", None),
            ("changed_path_after_observer_durable", None),
            ("changed_path_after_filesystem_applied", None),
            ("changed_path_after_ref_publish", None),
            ("changed_path_after_ref_publish", Some("missing")),
            ("changed_path_after_ref_publish", Some("truncated")),
            ("changed_path_after_ref_publish", Some("replaced")),
            ("changed_path_after_ref_publish", Some("corrupt")),
            ("changed_path_after_intent_publish", None),
            ("changed_path_after_recovery_commit", None),
        ]
        .into_iter()
        .enumerate()
        {
            let fixture = Fixture::new(0x70 + u8::try_from(index).unwrap()).unwrap();
            let label = tamper.map_or_else(|| phase.to_string(), |kind| format!("{phase}-{kind}"));
            let ready = fixture.db.db_dir.join("tmp").join(format!("{label}.ready"));
            let mut child = Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "db::change_ledger::recovery::harness::changed_path_subprocess_crash_helper",
                    "--nocapture",
                ])
                .env("RUST_TEST_THREADS", "1")
                .env("TRAIL_TEST_CRASH_AT", phase)
                .env("TRAIL_TEST_CRASH_READY", &ready)
                .env(
                    "TRAIL_TEST_CHANGED_PATH_CRASH_WORKSPACE",
                    &fixture.db.workspace_root,
                )
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            wait_for_crash_handshake(&mut child, &ready, phase);
            child.kill().unwrap();
            let _ = child.wait().unwrap();

            if let Some(tamper) = tamper {
                let segment_path: String = fixture
                    .db
                    .conn
                    .query_row(
                        "SELECT segment_path FROM changed_path_observer_segments
                         WHERE scope_id=?1 AND state='sealed' ORDER BY first_sequence LIMIT 1",
                        [fixture.expected.scope_id.to_text()],
                        |row| row.get(0),
                    )
                    .unwrap();
                let path = fixture
                    .db
                    .db_dir
                    .join("observer-segments")
                    .join(fixture.expected.scope_id.to_text())
                    .join(segment_path);
                match tamper {
                    "missing" => std::fs::remove_file(path).unwrap(),
                    "truncated" => {
                        let file = std::fs::OpenOptions::new().write(true).open(path).unwrap();
                        let len = file.metadata().unwrap().len();
                        file.set_len(len.saturating_sub(1)).unwrap();
                        file.sync_all().unwrap();
                    }
                    "replaced" => {
                        std::fs::remove_file(&path).unwrap();
                        std::fs::write(path, b"replacement segment").unwrap();
                    }
                    "corrupt" => {
                        use std::io::{Seek, Write};
                        let mut file = std::fs::OpenOptions::new()
                            .read(true)
                            .write(true)
                            .open(path)
                            .unwrap();
                        file.seek(std::io::SeekFrom::End(-1)).unwrap();
                        file.write_all(&[0xff]).unwrap();
                        file.sync_all().unwrap();
                    }
                    _ => unreachable!(),
                }
            }

            let reopened = Trail::open(&fixture.db.workspace_root).unwrap();
            let ledger = reopened.changed_path_ledger();
            let _ = recover_scope(&ledger, &fixture.expected);
            let fsck = reopened.fsck().unwrap();
            assert!(
                fsck.errors.is_empty(),
                "fsck failed after {phase}: {:?}",
                fsck.errors
            );
            let state: (String, i64, String) = reopened
                .conn
                .query_row(
                    "SELECT trust_state,ref_generation,baseline_root_id
                     FROM changed_path_scopes WHERE scope_id=?1",
                    [fixture.expected.scope_id.to_text()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .unwrap();
            if tamper.is_some() {
                assert_eq!(state.0, "untrusted_gap", "tampered proof stayed clean");
                assert_eq!(state.2, fixture.expected.baseline_root.0);
            } else if phase == "changed_path_after_object_graph" {
                assert_eq!(state.0, "trusted", "object-only boundary changed trust");
                assert_eq!(state.2, fixture.expected.baseline_root.0);
            } else if matches!(
                phase,
                "changed_path_after_intent_prepare"
                    | "changed_path_after_filesystem_write"
                    | "changed_path_after_observer_durable"
                    | "changed_path_after_filesystem_applied"
            ) {
                assert_eq!(state.0, "untrusted_gap", "false clean after {phase}");
                assert_eq!(state.2, fixture.expected.baseline_root.0);
            } else {
                assert_eq!(
                    state.0, "trusted",
                    "recoverable publication failed at {phase}"
                );
                assert_eq!(
                    u64::try_from(state.1).unwrap(),
                    fixture.expected.ref_generation + 1
                );
            }
        }
    }

    #[test]
    fn backup_restore_subprocess_crash_helper() {
        let Some(mode) = std::env::var_os("TRAIL_TEST_BACKUP_RESTORE_CRASH_MODE") else {
            return;
        };
        let workspace = std::path::PathBuf::from(
            std::env::var_os("TRAIL_TEST_BACKUP_RESTORE_WORKSPACE").unwrap(),
        );
        let backup =
            std::path::PathBuf::from(std::env::var_os("TRAIL_TEST_BACKUP_RESTORE_BACKUP").unwrap());
        if mode == "backup" {
            Trail::open(&workspace)
                .unwrap()
                .create_backup(&backup, true)
                .unwrap();
        } else {
            Trail::restore_backup(&workspace, &backup, true).unwrap();
        }
        panic!("backup/restore crash helper passed its requested crash point");
    }

    #[test]
    fn subprocess_kill_preserves_old_or_new_backup_and_restore_tree() {
        for phase in [
            "backup_restore_after_staging_sync",
            "backup_restore_after_atomic_exchange",
        ] {
            let fixture = Fixture::new(0x79).unwrap();
            std::fs::write(
                fixture.db.workspace_root.join(".trailignore"),
                b"old-backup\n",
            )
            .unwrap();
            fixture.db.create_backup(&fixture.backup, false).unwrap();
            std::fs::write(
                fixture.db.workspace_root.join(".trailignore"),
                b"new-backup\n",
            )
            .unwrap();
            run_backup_restore_child(
                "backup",
                phase,
                &fixture.db.workspace_root,
                &fixture.backup,
                &fixture
                    .db
                    .db_dir
                    .join("tmp")
                    .join(format!("backup-{phase}.ready")),
            );
            let verification = Trail::verify_backup(&fixture.backup).unwrap();
            assert!(
                verification.valid,
                "backup invalid after {phase}: {:?}",
                verification.errors
            );
            let ignore = std::fs::read(fixture.backup.join(".trailignore")).unwrap();
            assert!(ignore == b"old-backup\n" || ignore == b"new-backup\n");
        }

        for phase in [
            "restore_after_ledger_rotation",
            "restore_after_staged_workdir_rewrite",
            "restore_after_staged_recovery",
            "restore_after_staged_checkpoint",
            "restore_after_staged_sync",
            "backup_restore_after_staging_sync",
            "restore_after_policy_staging",
            "restore_after_policy_exchange_before_marker",
            "restore_after_policy_publication",
            "backup_restore_after_atomic_exchange",
            "restore_after_trail_publication",
            "restore_during_rollback",
            "restore_during_finalization",
            "restore_before_retained_cleanup",
            "restore_after_retained_cleanup",
        ] {
            let mut fixture = Fixture::new(0x7b).unwrap();
            fixture
                .db
                .spawn_lane("restore-crash-lane", Some("main"), true, None, None)
                .unwrap();
            std::fs::write(
                fixture.db.workspace_root.join(".trailignore"),
                b"new-policy-generation\n",
            )
            .unwrap();
            fixture.db.create_backup(&fixture.backup, false).unwrap();
            let destination = fixture._root.path().join(format!("restore-{phase}"));
            std::fs::write(destination.join("live.txt"), b"old live workspace\n").unwrap_or_else(
                |_| {
                    std::fs::create_dir_all(&destination).unwrap();
                    std::fs::write(destination.join("live.txt"), b"old live workspace\n").unwrap();
                },
            );
            Trail::init(&destination, "main", InitImportMode::WorkingTree, false).unwrap();
            std::fs::write(destination.join(".trail/live-marker"), b"old\n").unwrap();
            std::fs::write(destination.join(".trailignore"), b"old-policy-generation\n").unwrap();
            run_backup_restore_child(
                "restore",
                phase,
                &destination,
                &fixture.backup,
                &destination.join(format!("{phase}.ready")),
            );
            let old_is_intact = destination.join(".trail/live-marker").is_file();
            let new_is_intact = Trail::open(&destination)
                .and_then(|db| db.fsck())
                .is_ok_and(|report| report.errors.is_empty());
            assert!(
                old_is_intact || new_is_intact,
                "neither old nor restored tree survived {phase}"
            );
            let policy = std::fs::read(destination.join(".trailignore")).unwrap();
            assert_eq!(
                policy,
                if old_is_intact {
                    b"old-policy-generation\n".as_slice()
                } else {
                    b"new-policy-generation\n".as_slice()
                },
                "restore exposed a mixed DB/policy generation after {phase}"
            );
            if !old_is_intact {
                let restored = Trail::open(&destination).unwrap();
                let lane = restored.lane_details("restore-crash-lane").unwrap();
                let workdir = lane.branch.workdir.unwrap();
                let canonical_destination = destination.canonicalize().unwrap();
                assert!(
                    workdir.starts_with(
                        &canonical_destination
                            .join(".trail")
                            .to_string_lossy()
                            .to_string()
                    ),
                    "restored workdir was not rewritten after {phase}: {workdir}"
                );
                assert!(std::path::Path::new(&workdir).is_dir());
                let identity: (String, String) = restored
                    .conn
                    .query_row(
                        "SELECT trust_state,filesystem_identity FROM changed_path_scopes
                         WHERE retired_at IS NULL LIMIT 1",
                        [],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap();
                assert_eq!(identity.0, "untrusted_gap");
                assert!(!identity.1.is_empty());
            }
        }
    }

    #[test]
    fn backup_create_supports_a_missing_output_parent() {
        let fixture = Fixture::new(0x7d).unwrap();
        let backup = fixture._root.path().join("new-parent/backup");

        fixture.db.create_backup(&backup, false).unwrap();

        let verification = Trail::verify_backup(&backup).unwrap();
        assert!(verification.valid, "{:?}", verification.errors);
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn restore_supports_non_utf8_destination_with_fresh_identity() {
        use std::os::unix::ffi::OsStringExt;

        let fixture = Fixture::new(0x7f).unwrap();
        fixture.db.create_backup(&fixture.backup, false).unwrap();
        let destination = fixture
            ._root
            .path()
            .join(std::ffi::OsString::from_vec(b"restore-\xff".to_vec()));
        Trail::restore_backup(&destination, &fixture.backup, false).unwrap();
        let restored = Trail::open(&destination).unwrap();
        let row: (String, String) = restored
            .conn
            .query_row(
                "SELECT scope_root,filesystem_identity FROM changed_path_scopes",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!(row.0.starts_with("os-bytes:"));
        assert!(!row.1.is_empty());
    }

    fn run_backup_restore_child(
        mode: &str,
        phase: &str,
        workspace: &std::path::Path,
        backup: &std::path::Path,
        ready: &std::path::Path,
    ) {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "db::change_ledger::recovery::harness::backup_restore_subprocess_crash_helper",
                "--nocapture",
            ])
            .env("RUST_TEST_THREADS", "1")
            .env("TRAIL_TEST_CRASH_AT", phase)
            .env("TRAIL_TEST_CRASH_READY", ready)
            .env("TRAIL_TEST_BACKUP_RESTORE_CRASH_MODE", mode)
            .env("TRAIL_TEST_BACKUP_RESTORE_WORKSPACE", workspace)
            .env("TRAIL_TEST_BACKUP_RESTORE_BACKUP", backup)
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if phase == "restore_during_rollback" {
            command.env("TRAIL_TEST_RESTORE_FORCE_ROLLBACK", "1");
        }
        let mut child = command.spawn().unwrap();
        wait_for_crash_handshake(&mut child, ready, phase);
        child.kill().unwrap();
        let _ = child.wait().unwrap();
    }

    fn wait_for_crash_handshake(
        child: &mut std::process::Child,
        ready: &std::path::Path,
        phase: &str,
    ) {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if ready.is_file() {
                return;
            }
            if let Some(status) = child.try_wait().unwrap() {
                panic!("changed-path crash helper exited at {phase} before handshake: {status}");
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        panic!("timed out waiting for changed-path crash helper at {phase}");
    }

    fn run_real_crash_scenario(workspace: &std::path::Path) -> Result<()> {
        let mut db = Trail::open(workspace)?;
        let scope_id: String = db.conn.query_row(
            "SELECT scope_id FROM changed_path_scopes ORDER BY created_at LIMIT 1",
            [],
            |row| row.get(0),
        )?;
        let expected = load_expected_scope(&db.conn, &scope_id)?;
        let baseline = db.get_ref(&expected.ref_name)?;

        let target_path = workspace.join("crash-target.txt");
        std::fs::write(&target_path, b"durable target graph and filesystem write\n")?;
        db.record(
            None,
            Some("crash target graph".into()),
            Actor::system(),
            false,
        )?;
        let target_ref = db.get_ref(&expected.ref_name)?;
        db.conn.execute(
            "UPDATE refs SET change_id=?1,root_id=?2,operation_id=?3,generation=?4,updated_at=?5
             WHERE name=?6",
            params![
                baseline.change_id.0,
                baseline.root_id.0,
                baseline.operation_id.0,
                baseline.generation,
                now_ts(),
                baseline.name,
            ],
        )?;
        crate::db::util::write_ref_file(
            &db.db_dir,
            &baseline.name,
            &baseline.change_id,
            &baseline.root_id,
            &baseline.operation_id,
            baseline.generation,
        )?;
        std::fs::remove_file(&target_path)?;
        durable_intent_barrier(&db.conn)?;
        crate::db::util::test_crash_point("changed_path_after_object_graph");

        let target = IntentTarget {
            change_id: target_ref.change_id.clone(),
            root_id: target_ref.root_id.clone(),
            operation_id: Some(target_ref.operation_id.clone()),
        };
        let ledger = db.changed_path_ledger();
        let intent = prepare_intent(
            &ledger,
            &expected,
            IntentProducer::Checkout,
            &target,
            &Fixture::evidence("crash-target.txt"),
        )?;

        std::fs::write(&target_path, b"durable target graph and filesystem write\n")?;
        OpenOptions::new()
            .read(true)
            .open(&target_path)?
            .sync_all()?;
        crate::db::util::test_crash_point("changed_path_after_filesystem_write");

        let provider_id: String = db.conn.query_row(
            "SELECT provider_id FROM changed_path_scopes WHERE scope_id=?1",
            [&scope_id],
            |row| row.get(0),
        )?;
        let owner_token = [0x71_u8; 32];
        let segment_dir = db
            .db_dir
            .join("observer-segments")
            .join(expected.scope_id.to_text());
        let mut writer = super::super::SegmentWriter::acquire(
            &db.db_dir.join(crate::db::DB_RELATIVE_PATH),
            &segment_dir,
            expected.scope_id,
            expected.epoch,
            owner_token,
            &provider_id,
            b"cursor-1".to_vec(),
            Duration::from_secs(600),
        )?;
        writer.append(&[super::super::ObserverRecord {
            sequence: 1,
            source: EvidenceSource::Observer,
            path: LedgerPath::parse("crash-target.txt")?,
            flags: EvidenceFlags::CONTENT,
            provider_cursor: b"cursor-end".to_vec(),
        }])?;
        let durable = writer.flush_durable()?;
        writer.rotate()?;
        drop(writer);
        db.conn.execute(
            "UPDATE changed_path_observer_owners SET provider_identity=?1,fence_nonce=?2
             WHERE scope_id=?3",
            params![
                hex::encode(&expected.provider_identity),
                b"crash-fence".as_slice(),
                scope_id,
            ],
        )?;
        db.conn.execute(
            "UPDATE changed_path_observer_segments SET folded_end_offset=durable_end_offset
             WHERE scope_id=?1 AND segment_id=?2",
            params![scope_id, durable.segment_id],
        )?;
        db.conn.execute(
            "UPDATE changed_path_scopes SET provider_cursor=?1,durable_offset=?2,folded_offset=?2
             WHERE scope_id=?3",
            params![
                durable.provider_cursor,
                sql_u64(durable.durable_end_offset, "crash durable offset")?,
                scope_id,
            ],
        )?;
        db.conn.execute(
            "INSERT INTO changed_path_entries(scope_id,normalized_path,event_flags,source_mask,
                 first_sequence,last_sequence,provider_id,provider_sequence,intent_id,created_at,updated_at)
             VALUES(?1,'crash-target.txt',?2,1,1,1,?3,1,NULL,?4,?4)",
            params![scope_id, EvidenceFlags::CONTENT.0, provider_id, now_ts()],
        )?;
        durable_intent_barrier(&db.conn)?;
        crate::db::util::test_crash_point("changed_path_after_observer_durable");

        let segment = db.conn.query_row(
            "SELECT segment_id,segment_hash,segment_path,durable_end_offset,folded_end_offset
             FROM changed_path_observer_segments
             WHERE scope_id=?1 AND state='sealed'",
            [&scope_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )?;
        let hash: [u8; 32] = hex::decode(segment.1)
            .map_err(|error| Error::Corrupt(error.to_string()))?
            .try_into()
            .map_err(|_| Error::Corrupt("invalid crash segment hash".into()))?;
        let proof = QualifiedFilesystemProof {
            scope_id: expected.scope_id,
            epoch: expected.epoch,
            expected_root_id: expected.baseline_root.clone(),
            scope_root_identity: expected.filesystem_identity.clone(),
            filesystem_identity: expected.filesystem_identity.clone(),
            provider_id,
            provider_identity: expected.provider_identity.clone(),
            observer_owner_token: hex::encode(owner_token),
            owner_fence_nonce: Some(b"crash-fence".to_vec()),
            durable_segment_id: segment.0,
            durable_segment_hash: hash,
            segment_directory: format!("observer-segments/{}", expected.scope_id.to_text()),
            segment_path: segment.2,
            start_cursor: Some(b"cursor-1".to_vec()),
            end_cursor: b"cursor-end".to_vec(),
            start_sequence: 1,
            end_cut: EvidenceCut {
                source: EvidenceSource::Observer,
                sequence: 1,
                durable_offset: durable.durable_end_offset,
                folded_offset: durable.durable_end_offset,
            },
            segment_durable_offset: db_u64(segment.3, "crash segment durable")?,
            segment_folded_offset: db_u64(segment.4, "crash segment folded")?,
            verified_paths: 1,
            verified_prefixes: 0,
            complete_root_interval: true,
            complete_policy_interval: true,
            persisted_evidence_through_end: true,
        };
        mark_filesystem_applied(&ledger, &expected, &intent, &proof)?;

        db.conn.execute(
            "UPDATE refs SET change_id=?1,root_id=?2,operation_id=?3,generation=?4,updated_at=?5
             WHERE name=?6",
            params![
                target_ref.change_id.0,
                target_ref.root_id.0,
                target_ref.operation_id.0,
                target_ref.generation,
                now_ts(),
                target_ref.name,
            ],
        )?;
        crate::db::util::write_ref_file(
            &db.db_dir,
            &target_ref.name,
            &target_ref.change_id,
            &target_ref.root_id,
            &target_ref.operation_id,
            target_ref.generation,
        )?;
        durable_intent_barrier(&db.conn)?;
        crate::db::util::test_crash_point("changed_path_after_ref_publish");
        publish_intent(&ledger, &expected, &intent)?;
        recover_scope(&ledger, &expected)?;
        Ok(())
    }

    fn load_expected_scope(conn: &Connection, scope_id: &str) -> Result<ExpectedScope> {
        let row = conn.query_row(
            "SELECT epoch,ref_name,ref_generation,baseline_root_id,policy_fingerprint,
                    policy_dependency_generation,filesystem_identity,provider_identity
             FROM changed_path_scopes WHERE scope_id=?1",
            [scope_id],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )?;
        let scope: [u8; 32] = hex::decode(scope_id)
            .map_err(|error| Error::Corrupt(error.to_string()))?
            .try_into()
            .map_err(|_| Error::Corrupt("invalid crash scope id".into()))?;
        let policy: [u8; 32] = hex::decode(row.4)
            .map_err(|error| Error::Corrupt(error.to_string()))?
            .try_into()
            .map_err(|_| Error::Corrupt("invalid crash policy".into()))?;
        Ok(ExpectedScope {
            scope_id: ScopeId(scope),
            epoch: db_u64(row.0, "crash epoch")?,
            ref_name: row.1,
            ref_generation: db_u64(row.2, "crash ref generation")?,
            baseline_root: ObjectId(row.3),
            policy_fingerprint: policy,
            policy_generation: db_u64(row.5, "crash policy generation")?,
            filesystem_identity: hex::decode(row.6)
                .map_err(|error| Error::Corrupt(error.to_string()))?,
            provider_identity: hex::decode(row.7)
                .map_err(|error| Error::Corrupt(error.to_string()))?,
        })
    }

    pub(super) fn rejects_unqualified_or_stale_filesystem_proof() -> Result<()> {
        let fixture = Fixture::new(0x68)?;
        let target = fixture.target("stale-proof", fixture.expected.baseline_root.clone());
        let ledger = fixture.db.changed_path_ledger();
        let intent = prepare_intent(
            &ledger,
            &fixture.expected,
            IntentProducer::Checkout,
            &target,
            &Fixture::evidence("stale-proof.rs"),
        )?;
        mark_filesystem_applied(
            &ledger,
            &fixture.expected,
            &intent,
            &fixture.qualified_proof(11, "stale-proof.rs")?,
        )?;
        fixture.db.conn.execute(
            "UPDATE changed_path_observer_segments SET segment_hash=?1 WHERE scope_id=?2",
            params![
                hex::encode([0xff_u8; 32]),
                fixture.expected.scope_id.to_text()
            ],
        )?;
        fixture.advance_ref_to(&target)?;

        let decisions = recover_scope(&ledger, &fixture.expected)?;
        if decisions == vec![(intent.clone(), RecoveryDecision::FinishPublication)] {
            return Err(Error::Corrupt(
                "an unqualified filesystem cut authorized baseline publication".into(),
            ));
        }
        let state: String = fixture.db.conn.query_row(
            "SELECT trust_state FROM changed_path_scopes WHERE scope_id=?1",
            [fixture.expected.scope_id.to_text()],
            |row| row.get(0),
        )?;
        if state != "untrusted_gap" {
            return Err(Error::Corrupt(format!(
                "unqualified proof did not fail closed: {state}"
            )));
        }
        Ok(())
    }

    pub(super) fn rejects_metadata_only_proof_without_sidecar() -> Result<()> {
        let fixture = Fixture::new(0x80)?;
        let target = fixture.target("missing-sidecar", fixture.expected.baseline_root.clone());
        let ledger = fixture.db.changed_path_ledger();
        let intent = prepare_intent(
            &ledger,
            &fixture.expected,
            IntentProducer::Checkout,
            &target,
            &Fixture::evidence("missing-sidecar.rs"),
        )?;
        let proof = fixture.qualified_proof(1, "missing-sidecar.rs")?;
        std::fs::remove_file(
            fixture
                .db
                .db_dir
                .join(&proof.segment_directory)
                .join(&proof.segment_path),
        )?;

        if mark_filesystem_applied(&ledger, &fixture.expected, &intent, &proof).is_ok() {
            return Err(Error::Corrupt(
                "metadata-only intent proof was accepted without a segment sidecar".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn ambiguous_recovery_blocks_next_intent() -> Result<()> {
        let fixture = Fixture::new(0x69)?;
        let first_target =
            fixture.target("ambiguous-first", fixture.expected.baseline_root.clone());
        let ledger = fixture.db.changed_path_ledger();
        prepare_intent(
            &ledger,
            &fixture.expected,
            IntentProducer::Checkout,
            &first_target,
            &Fixture::evidence("first.rs"),
        )?;
        recover_scope(&ledger, &fixture.expected)?;

        let second_target =
            fixture.target("ambiguous-second", fixture.expected.baseline_root.clone());
        if prepare_intent(
            &ledger,
            &fixture.expected,
            IntentProducer::Checkout,
            &second_target,
            &Fixture::evidence("second.rs"),
        )
        .is_ok()
        {
            return Err(Error::Corrupt(
                "a new intent was prepared while full reconciliation was required".into(),
            ));
        }
        Ok(())
    }

    pub(super) fn backup_restore_rotation() -> Result<()> {
        let fixture = Fixture::new(0x66)?;
        fixture.db.create_backup(&fixture.backup, false)?;
        let backup_conn = Connection::open(fixture.backup.join("index/trail.sqlite"))?;
        let backup_state: String =
            backup_conn.query_row("SELECT trust_state FROM changed_path_scopes", [], |row| {
                row.get(0)
            })?;
        if backup_state != "untrusted_gap" {
            return Err(Error::Corrupt("backup retained trusted scope".into()));
        }
        drop(backup_conn);
        Trail::restore_backup(&fixture.restore, &fixture.backup, false)?;
        let restored = Connection::open(fixture.restore.join(".trail/index/trail.sqlite"))?;
        let row = restored.query_row(
            "SELECT epoch,trust_state,provider_identity,provider_cursor,durable_offset,
                    folded_offset,filesystem_identity FROM changed_path_scopes",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<Vec<u8>>>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, String>(6)?,
                ))
            },
        )?;
        if row.0 != 2
            || row.1 != "untrusted_gap"
            || row.2.is_some()
            || row.3.is_some()
            || row.4 != 0
            || row.5 != 0
            || row.6 == hex::encode(&fixture.expected.filesystem_identity)
        {
            return Err(Error::Corrupt(format!(
                "restore did not rotate ledger identity: {row:?}"
            )));
        }
        drop(restored);
        Trail::restore_backup(&fixture.restore, &fixture.backup, true)?;
        let restored_again = Connection::open(fixture.restore.join(".trail/index/trail.sqlite"))?;
        let repeated: (i64, i64, String) = restored_again.query_row(
            "SELECT epoch,continuity_generation,filesystem_identity
             FROM changed_path_scopes",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
        if repeated.0 <= row.0 || repeated.1 <= 1 || repeated.2 == row.6 {
            return Err(Error::Corrupt(format!(
                "repeated restore reused its ledger incarnation: first={row:?} repeated={repeated:?}"
            )));
        }
        Ok(())
    }

    pub(super) fn backup_overwrite_failure_preserves_previous() -> Result<()> {
        let fixture = Fixture::new(0x6a)?;
        fixture.db.create_backup(&fixture.backup, false)?;
        let previous_manifest = std::fs::read(fixture.backup.join("manifest.json"))?;
        std::fs::remove_file(fixture.db.db_dir.join(crate::db::CONFIG_FILE))?;
        if fixture.db.create_backup(&fixture.backup, true).is_ok() {
            return Err(Error::Corrupt(
                "backup overwrite unexpectedly succeeded with a missing source config".into(),
            ));
        }
        let retained = std::fs::read(fixture.backup.join("manifest.json"))?;
        if retained != previous_manifest {
            return Err(Error::Corrupt(
                "failed backup overwrite did not retain the prior valid tree".into(),
            ));
        }
        Ok(())
    }
}

#[cfg(debug_assertions)]
pub(crate) fn run_acknowledgement_race() -> std::result::Result<(), String> {
    harness::acknowledgement_race().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_gc_root_lifecycle() -> std::result::Result<(), String> {
    harness::gc_root_lifecycle().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_crash_matrix() -> std::result::Result<(), String> {
    harness::crash_matrix().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_backup_restore_rotation() -> std::result::Result<(), String> {
    harness::backup_restore_rotation().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_qualified_proof_revalidation() -> std::result::Result<(), String> {
    harness::rejects_unqualified_or_stale_filesystem_proof().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_ambiguous_recovery_gate() -> std::result::Result<(), String> {
    harness::ambiguous_recovery_blocks_next_intent().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_backup_overwrite_rollback() -> std::result::Result<(), String> {
    harness::backup_overwrite_failure_preserves_previous().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_deletion_post_verification_substitution_rejection(
) -> std::result::Result<(), String> {
    harness::deletion_name_substitution_after_verification_fails_closed()
        .map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_deletion_post_quarantine_verification_substitution_rejection(
) -> std::result::Result<(), String> {
    harness::deletion_substitution_after_quarantine_verification_fails_closed()
        .map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_deletion_retry_hostile_quarantine_replacement_rejection(
) -> std::result::Result<(), String> {
    harness::deletion_retry_rejects_hostile_quarantine_replacement()
        .map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_deletion_normal_retry_idempotence() -> std::result::Result<(), String> {
    harness::deletion_normal_retry_is_durably_idempotent().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_retained_writer_quiescence() -> std::result::Result<(), String> {
    harness::retirement_requires_retained_writer_quiescence().map_err(|error| error.to_string())
}

#[cfg(all(debug_assertions, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn run_orphan_quarantine_substitution_rejection() -> std::result::Result<(), String> {
    harness::orphan_quarantine_substitution_fails_closed().map_err(|error| error.to_string())
}

#[cfg(all(debug_assertions, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn run_empty_orphan_quarantine_rejection() -> std::result::Result<(), String> {
    harness::empty_orphan_quarantine_is_retained_and_rejected().map_err(|error| error.to_string())
}

#[cfg(all(debug_assertions, any(target_os = "linux", target_os = "macos")))]
pub(crate) fn run_no_orphan_quarantine_allocation() -> std::result::Result<(), String> {
    harness::no_orphan_allocates_fresh_quarantine_authority().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_deletion_quiesced_missing_quarantine_rejection() -> std::result::Result<(), String>
{
    harness::deletion_quiesced_retry_rejects_missing_quarantine().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_deletion_quiesced_reappeared_original_rejection(
) -> std::result::Result<(), String> {
    harness::deletion_quiesced_retry_rejects_reappeared_original()
        .map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_restored_nullable_provider_lane_deletion() -> std::result::Result<(), String> {
    harness::restored_nullable_provider_lane_deletion().map_err(|error| error.to_string())
}

#[cfg(all(debug_assertions, unix))]
pub(crate) fn run_non_utf_database_path_mark_recover_and_retire() -> std::result::Result<(), String>
{
    harness::non_utf_database_path_mark_recover_and_retire().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_retirement_barrier() -> std::result::Result<(), String> {
    harness::retirement_path_and_reader_barrier().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_lane_deletion_retirement() -> std::result::Result<(), String> {
    harness::lane_deletion_retires_scope_first().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_missing_sidecar_rejection() -> std::result::Result<(), String> {
    harness::rejects_metadata_only_proof_without_sidecar().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_advanced_prefix_recovery() -> std::result::Result<(), String> {
    harness::authenticated_prefix_survives_later_observer_advance()
        .map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_exact_interval_bridge_rejection() -> std::result::Result<(), String> {
    harness::exact_path_aggregate_bridge_is_rejected().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_prefix_interval_bridge_rejection() -> std::result::Result<(), String> {
    harness::prefix_aggregate_bridge_is_rejected().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_valid_prefix_interval_recovery() -> std::result::Result<(), String> {
    harness::authenticated_prefix_interval_preserves_later_suffix()
        .map_err(|error| error.to_string())
}

#[cfg(all(debug_assertions, unix))]
pub(crate) fn run_mark_ancestor_substitution_rejection() -> std::result::Result<(), String> {
    harness::ancestor_substitution_at_mark_is_rejected().map_err(|error| error.to_string())
}

#[cfg(all(debug_assertions, unix))]
pub(crate) fn run_recovery_ancestor_substitution_rejection() -> std::result::Result<(), String> {
    harness::ancestor_substitution_at_recovery_is_rejected().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_deletion_parent_substitution_rejection() -> std::result::Result<(), String> {
    harness::deletion_parent_substitution_uses_retained_authority()
        .map_err(|error| error.to_string())
}

#[cfg(all(debug_assertions, unix))]
pub(crate) fn run_deletion_leaf_substitution_rejection() -> std::result::Result<(), String> {
    harness::deletion_leaf_substitution_fails_closed().map_err(|error| error.to_string())
}
