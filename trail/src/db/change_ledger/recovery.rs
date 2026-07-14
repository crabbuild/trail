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
            validate_qualified_filesystem_proof(&tx, expected, &intent, proof).is_ok()
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

pub(crate) fn retire_scope(conn: &Connection, expected: &ExpectedScope) -> Result<Vec<String>> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let now = now_ts();
    let scope_state = tx
        .query_row(
            "SELECT retired_at FROM changed_path_scopes
             WHERE scope_id=?1 AND epoch=?2 AND ref_name=?3 AND ref_generation=?4
               AND baseline_root_id=?5 AND filesystem_identity=?6 AND provider_identity=?7",
            params![
                expected.scope_id.to_text(),
                sql_u64(expected.epoch, "scope epoch")?,
                expected.ref_name,
                sql_u64(expected.ref_generation, "ref generation")?,
                expected.baseline_root.0,
                hex::encode(&expected.filesystem_identity),
                hex::encode(&expected.provider_identity),
            ],
            |row| row.get::<_, Option<i64>>(0),
        )
        .optional()?;
    let Some(retired_at) = scope_state else {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "scope retirement CAS failed".into(),
            command: "trail status".into(),
        });
    };
    let paths = tx
        .prepare(
            "SELECT segment_path FROM changed_path_observer_segments
             WHERE scope_id=?1 AND epoch=?2 ORDER BY first_sequence",
        )?
        .query_map(
            params![
                expected.scope_id.to_text(),
                sql_u64(expected.epoch, "scope epoch")?
            ],
            |row| row.get(0),
        )?
        .collect::<std::result::Result<Vec<String>, _>>()?;
    validate_retired_segment_paths(&paths)?;
    if retired_at.is_some() {
        tx.commit()?;
        durable_intent_barrier(conn)?;
        return Ok(paths);
    }
    let revoked = tx.execute(
        "UPDATE changed_path_observer_owners SET lease_state='revoked',updated_at=?1
         WHERE scope_id=?2 AND epoch=?3 AND lease_state='active'",
        params![
            now,
            expected.scope_id.to_text(),
            sql_u64(expected.epoch, "scope epoch")?
        ],
    )?;
    let changed = tx.execute(
        "UPDATE changed_path_scopes SET retired_at=?1,trust_state='untrusted_gap',
             trust_reason='scope_retired',continuity_generation=continuity_generation+?9,
             observer_owner_token=NULL,observer_heartbeat_at=NULL,updated_at=?1
         WHERE scope_id=?2 AND epoch=?3 AND ref_name=?4 AND ref_generation=?5
           AND baseline_root_id=?6 AND filesystem_identity=?7 AND provider_identity=?8
           AND retired_at IS NULL",
        params![
            now,
            expected.scope_id.to_text(),
            sql_u64(expected.epoch, "scope epoch")?,
            expected.ref_name,
            sql_u64(expected.ref_generation, "ref generation")?,
            expected.baseline_root.0,
            hex::encode(&expected.filesystem_identity),
            hex::encode(&expected.provider_identity),
            i64::from(revoked == 0)
        ],
    )?;
    if changed != 1 {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "scope retirement CAS failed".into(),
            command: "trail status".into(),
        });
    }
    tx.execute(
        "UPDATE changed_path_observer_segments SET state='retired',updated_at=?1
         WHERE scope_id=?2 AND epoch=?3",
        params![
            now,
            expected.scope_id.to_text(),
            sql_u64(expected.epoch, "scope epoch")?
        ],
    )?;
    tx.commit()?;
    durable_intent_barrier(conn)?;
    Ok(paths)
}

pub(crate) fn retire_deletion_scopes(
    conn: &Connection,
    owner_ids: &[&str],
    scope_roots: &[&str],
) -> Result<Vec<String>> {
    let rows = {
        let mut statement = conn.prepare(
            "SELECT scope_id,epoch,ref_name,ref_generation,baseline_root_id,
                    policy_fingerprint,policy_dependency_generation,filesystem_identity,
                    provider_identity,owner_id,scope_root
             FROM changed_path_scopes
             WHERE scope_kind IN ('materialized_lane','workspace_view')
             ORDER BY scope_id",
        )?;
        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        rows
    };
    let mut retired_paths = Vec::new();
    for row in rows {
        if !owner_ids.contains(&row.9.as_str()) && !scope_roots.contains(&row.10.as_str()) {
            continue;
        }
        let provider_identity = row.8.ok_or_else(|| Error::ChangeLedgerReconcileRequired {
            scope: row.0.clone(),
            state: "untrusted_gap".into(),
            reason: "deletion scope has no qualified provider identity".into(),
            command: "trail status".into(),
        })?;
        let scope_bytes = hex::decode(&row.0).map_err(|error| Error::Corrupt(error.to_string()))?;
        let policy_bytes =
            hex::decode(&row.5).map_err(|error| Error::Corrupt(error.to_string()))?;
        let expected = ExpectedScope {
            scope_id: super::ScopeId(
                scope_bytes
                    .try_into()
                    .map_err(|_| Error::Corrupt("invalid deletion scope id".into()))?,
            ),
            epoch: db_u64(row.1, "deletion scope epoch")?,
            ref_name: row.2,
            ref_generation: db_u64(row.3, "deletion ref generation")?,
            baseline_root: ObjectId(row.4),
            policy_fingerprint: policy_bytes
                .try_into()
                .map_err(|_| Error::Corrupt("invalid deletion policy fingerprint".into()))?,
            policy_generation: db_u64(row.6, "deletion policy generation")?,
            filesystem_identity: hex::decode(row.7)
                .map_err(|error| Error::Corrupt(error.to_string()))?,
            provider_identity: hex::decode(provider_identity)
                .map_err(|error| Error::Corrupt(error.to_string()))?,
        };
        retired_paths.extend(retire_scope(conn, &expected)?);
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
    use crate::db::change_ledger::{
        mark_filesystem_applied, prepare_intent, publish_intent, BaselineIdentity, EvidenceCut,
        EvidenceFlags, EvidenceSource, FilesystemIdentity, IntentEvidence, IntentProducer,
        IntentTarget, LedgerPath, PolicyIdentity, ProviderCapabilities, ProviderIdentity,
        QualifiedFilesystemProof, ScopeId, ScopeIdentity, ScopeKind,
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
            ChangedPathLedger::new(&db.conn).begin_scope(
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
            let provider_id: String = self.db.conn.query_row(
                "SELECT provider_id FROM changed_path_scopes WHERE scope_id=?1",
                [self.expected.scope_id.to_text()],
                |row| row.get(0),
            )?;
            let owner_token = format!("qualified-owner-{}", self.expected.scope_id.to_text());
            let fence_nonce = b"qualified-fence".to_vec();
            let segment_id = format!("qualified-segment-{}", self.expected.scope_id.to_text());
            let segment_path = format!("{segment_id}.cpl");
            let segment_hash = [self.expected.scope_id.0[0]; 32];
            let end_cursor = format!("cursor-{sequence}").into_bytes();
            let now = now_ts();
            self.insert_observer_event(path, sequence)?;
            self.db.conn.execute(
                "UPDATE changed_path_scopes SET provider_cursor=?1,durable_offset=100,
                     folded_offset=100 WHERE scope_id=?2",
                params![end_cursor, self.expected.scope_id.to_text()],
            )?;
            self.db.conn.execute(
                "INSERT INTO changed_path_observer_owners(
                     scope_id,epoch,owner_token,provider_id,provider_identity,lease_state,
                     fence_nonce,acquired_at,heartbeat_at,expires_at,error_state,error_at,updated_at)
                 VALUES(?1,?2,?3,?4,?5,'active',?6,?7,?7,?8,NULL,NULL,?7)",
                params![
                    self.expected.scope_id.to_text(),
                    sql_u64(self.expected.epoch, "fixture epoch")?,
                    owner_token,
                    provider_id,
                    hex::encode(&self.expected.provider_identity),
                    fence_nonce,
                    now,
                    now + 600,
                ],
            )?;
            self.db.conn.execute(
                "INSERT INTO changed_path_observer_segments(
                     scope_id,epoch,segment_id,log_format_version,owner_token,provider_id,
                     first_sequence,last_sequence,durable_end_offset,folded_end_offset,
                     previous_segment_id,previous_segment_hash,segment_hash,segment_path,state,
                     created_at,sealed_at,updated_at)
                 VALUES(?1,?2,?3,1,?4,?5,1,?6,100,100,NULL,NULL,?7,?8,'sealed',?9,?9,?9)",
                params![
                    self.expected.scope_id.to_text(),
                    sql_u64(self.expected.epoch, "fixture epoch")?,
                    segment_id,
                    owner_token,
                    provider_id,
                    sql_u64(sequence, "fixture sequence")?,
                    hex::encode(segment_hash),
                    segment_path,
                    now,
                ],
            )?;
            Ok(QualifiedFilesystemProof {
                scope_id: self.expected.scope_id,
                epoch: self.expected.epoch,
                expected_root_id: self.expected.baseline_root.clone(),
                scope_root_identity: self.expected.filesystem_identity.clone(),
                filesystem_identity: self.expected.filesystem_identity.clone(),
                provider_id,
                provider_identity: self.expected.provider_identity.clone(),
                observer_owner_token: owner_token,
                owner_fence_nonce: Some(fence_nonce),
                durable_segment_id: segment_id,
                durable_segment_hash: segment_hash,
                segment_path,
                start_cursor: Some(b"cursor-1".to_vec()),
                end_cursor,
                start_sequence: 1,
                end_cut: EvidenceCut {
                    source: EvidenceSource::Observer,
                    sequence,
                    durable_offset: 100,
                    folded_offset: 100,
                },
                segment_durable_offset: 100,
                segment_folded_offset: 100,
                verified_paths: 1,
                verified_prefixes: 0,
                complete_root_interval: true,
                complete_policy_interval: true,
                persisted_evidence_through_end: true,
            })
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
    }

    pub(super) fn acknowledgement_race() -> Result<()> {
        let fixture = Fixture::new(0x61)?;
        let target = fixture.target("ack", fixture.expected.baseline_root.clone());
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
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
            &ChangedPathLedger::new(&fixture.db.conn),
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
            &ChangedPathLedger::new(&prepared.db.conn),
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
        recover_scope(
            &ChangedPathLedger::new(&prepared.db.conn),
            &prepared.expected,
        )?;
        recover_scope(
            &ChangedPathLedger::new(&prepared.db.conn),
            &prepared.expected,
        )?;
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
        let now = now_ts();
        retired.db.conn.execute(
            "INSERT INTO changed_path_observer_owners(
                 scope_id,epoch,owner_token,provider_id,provider_identity,lease_state,
                 fence_nonce,acquired_at,heartbeat_at,expires_at,error_state,error_at,updated_at
             ) VALUES(?1,1,'retire-owner','provider','identity','active',NULL,?2,?2,?3,NULL,NULL,?2)",
            params![retired.expected.scope_id.to_text(), now, now + 60],
        )?;
        retired.db.conn.execute(
            "INSERT INTO changed_path_observer_segments(
                 scope_id,epoch,segment_id,log_format_version,owner_token,provider_id,
                 first_sequence,last_sequence,durable_end_offset,folded_end_offset,
                 previous_segment_id,previous_segment_hash,segment_hash,segment_path,state,
                 created_at,sealed_at,updated_at
             ) VALUES(?1,1,'retire-segment',1,'retire-owner','provider',1,NULL,0,0,
                      NULL,NULL,NULL,'retire-segment.cpl','open',?2,NULL,?2)",
            params![retired.expected.scope_id.to_text(), now],
        )?;
        let retired_paths = retire_scope(&retired.db.conn, &retired.expected)?;
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
        if retired_paths != vec!["retire-segment.cpl"]
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
            let ledger = ChangedPathLedger::new(&fixture.db.conn);
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
        if retire_scope(&malicious.db.conn, &malicious.expected).is_ok() {
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
        reader_fixture.db.conn.execute(
            "INSERT INTO changed_path_observer_segments(
                 scope_id,epoch,segment_id,log_format_version,owner_token,provider_id,
                 first_sequence,last_sequence,durable_end_offset,folded_end_offset,
                 previous_segment_id,previous_segment_hash,segment_hash,segment_path,state,
                 created_at,sealed_at,updated_at)
             VALUES(?1,1,'reader-segment',1,'owner','provider',1,1,1,1,NULL,NULL,?2,
                    'reader-segment.cpl','sealed',?3,?3,?3)",
            params![
                reader_fixture.expected.scope_id.to_text(),
                hex::encode([2_u8; 32]),
                now_ts()
            ],
        )?;
        let reader = Connection::open(reader_fixture.db.db_dir.join(crate::db::DB_RELATIVE_PATH))?;
        reader.execute_batch("BEGIN DEFERRED")?;
        let _: i64 = reader.query_row("SELECT COUNT(*) FROM changed_path_scopes", [], |row| {
            row.get(0)
        })?;
        if retire_scope(&reader_fixture.db.conn, &reader_fixture.expected).is_ok() {
            return Err(Error::Corrupt(
                "retirement returned deletion authority while an older reader was active".into(),
            ));
        }
        reader.execute_batch("COMMIT")?;
        let paths = retire_scope(&reader_fixture.db.conn, &reader_fixture.expected)?;
        if paths != vec!["reader-segment.cpl"] {
            return Err(Error::Corrupt(format!(
                "retirement retry did not return confined paths: {paths:?}"
            )));
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
        ChangedPathLedger::new(&db.conn).begin_scope(
            &scope,
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
            "UPDATE changed_path_observer_segments SET segment_path='lane-segment.cpl'
             WHERE scope_id=?1",
            [scope.scope_id.to_text()],
        )?;
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
        let retired: bool = db.conn.query_row(
            "SELECT retired_at IS NOT NULL FROM changed_path_scopes WHERE scope_id=?1",
            [scope.scope_id.to_text()],
            |row| row.get(0),
        )?;
        if !retired || workdir.exists() {
            return Err(Error::Corrupt(
                "lane scope was not retired before workdir deletion".into(),
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
    fn subprocess_kill_and_reopen_covers_intent_durability_boundaries() {
        for (index, phase) in [
            "changed_path_after_object_graph",
            "changed_path_after_intent_prepare",
            "changed_path_after_filesystem_write",
            "changed_path_after_observer_durable",
            "changed_path_after_filesystem_applied",
            "changed_path_after_ref_publish",
            "changed_path_after_intent_publish",
            "changed_path_after_recovery_commit",
        ]
        .into_iter()
        .enumerate()
        {
            let fixture = Fixture::new(0x70 + u8::try_from(index).unwrap()).unwrap();
            let ready = fixture.db.db_dir.join("tmp").join(format!("{phase}.ready"));
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

            let reopened = Trail::open(&fixture.db.workspace_root).unwrap();
            let ledger = ChangedPathLedger::new(&reopened.conn);
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
            if index == 0 {
                assert_eq!(state.0, "trusted", "object-only boundary changed trust");
                assert_eq!(state.2, fixture.expected.baseline_root.0);
            } else if index < 5 {
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
            "backup_restore_after_staging_sync",
            "backup_restore_after_atomic_exchange",
        ] {
            let fixture = Fixture::new(0x7b).unwrap();
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
        let mut child = Command::new(std::env::current_exe().unwrap())
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
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
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
        let ledger = ChangedPathLedger::new(&db.conn);
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
        let segment_dir = db.db_dir.join("observer-crash-segments");
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
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
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

    pub(super) fn ambiguous_recovery_blocks_next_intent() -> Result<()> {
        let fixture = Fixture::new(0x69)?;
        let first_target =
            fixture.target("ambiguous-first", fixture.expected.baseline_root.clone());
        let ledger = ChangedPathLedger::new(&fixture.db.conn);
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
pub(crate) fn run_retirement_barrier() -> std::result::Result<(), String> {
    harness::retirement_path_and_reader_barrier().map_err(|error| error.to_string())
}

#[cfg(debug_assertions)]
pub(crate) fn run_lane_deletion_retirement() -> std::result::Result<(), String> {
    harness::lane_deletion_retires_scope_first().map_err(|error| error.to_string())
}
