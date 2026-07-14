use std::collections::HashSet;

use rusqlite::{params, Connection, OptionalExtension, Transaction, TransactionBehavior};

use super::intent::{
    authoritative_ref_matches_target, db_u64, durable_intent_barrier, load_intent, sql_u64,
    stage_intent_evidence, IntentId, IntentState, PersistedIntent,
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
        let decision = if exact_scope
            && target_published
            && matches!(
                intent.state,
                IntentState::FilesystemApplied | IntentState::Published
            )
            && intent.verified_cut.is_some()
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

pub(crate) fn ledger_gc_roots(conn: &Connection) -> Result<HashSet<String>> {
    let mut roots = HashSet::new();
    let mut statement = conn.prepare(
        "SELECT target_root_id,target_operation_id FROM changed_path_intents
         WHERE lifecycle_state IN ('prepared','filesystem_applied','published')",
    )?;
    let rows = statement.query_map([], |row| {
        Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
    })?;
    for row in rows {
        let (root, operation) = row?;
        roots.insert(root);
        if let Some(operation) = operation {
            roots.insert(operation);
        }
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
) -> Result<()> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let now = now_ts();
    tx.execute(
        "UPDATE changed_path_intents SET lifecycle_state='aborted',
             failure_reason='restore_rotated_scope_identity',updated_at=?1
         WHERE lifecycle_state IN ('prepared','filesystem_applied','published')",
        [now],
    )?;
    tx.execute("DELETE FROM changed_path_observer_owners", [])?;
    tx.execute("DELETE FROM changed_path_observer_segments", [])?;
    tx.execute(
        "UPDATE changed_path_scopes SET epoch=epoch+1,scope_root=?1,
             filesystem_identity=?2,scope_root_identity=?2,
             provider_id=NULL,provider_identity=NULL,provider_cursor=NULL,provider_fence=NULL,
             observer_owner_token=NULL,observer_heartbeat_at=NULL,
             durable_offset=0,folded_offset=0,trust_state='untrusted_gap',
             trust_reason='restored_filesystem_identity_rotated',
             continuity_generation=continuity_generation+1,updated_at=?3
         WHERE retired_at IS NULL",
        params![scope_root, hex::encode(filesystem_identity), now],
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn retire_scope(conn: &Connection, expected: &ExpectedScope) -> Result<Vec<String>> {
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    let now = now_ts();
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
            .is_none_or(|cut| cut.filesystem_identity == expected.filesystem_identity)
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

    use super::*;
    use crate::db::change_ledger::{
        mark_filesystem_applied, prepare_intent, publish_intent, BaselineIdentity, EvidenceCut,
        EvidenceFlags, EvidenceSource, FilesystemIdentity, IntentEvidence, IntentProducer,
        IntentTarget, LedgerPath, PolicyIdentity, ProviderCapabilities, ProviderIdentity, ScopeId,
        ScopeIdentity, ScopeKind, VerifiedFilesystemCut,
    };
    use crate::db::{InitImportMode, Trail, ROOT_OBJECT_VERSION, WORKTREE_ROOT_KIND};
    use crate::model::WorktreeRoot;
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

        fn cut(&self, sequence: u64) -> VerifiedFilesystemCut {
            VerifiedFilesystemCut {
                observer_cut: EvidenceCut {
                    source: EvidenceSource::Observer,
                    sequence,
                    durable_offset: 0,
                    folded_offset: 0,
                },
                verified_paths: 1,
                filesystem_identity: self.expected.filesystem_identity.clone(),
            }
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
                 VALUES(?1,?2,?3,1,?4,?4,'observer',?4,NULL,?5,?5)",
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
        mark_filesystem_applied(&ledger, &fixture.expected, &intent, &fixture.cut(7))?;
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
        let target_change = ChangeId("change-recovery-gc".into());
        let root = WorktreeRoot {
            version: ROOT_OBJECT_VERSION,
            path_map_root: None,
            file_index_map_root: None,
            case_fold_map_root: None,
            file_count: 0,
            total_text_bytes: 0,
            created_by: target_change.clone(),
        };
        let target_root = fixture
            .db
            .put_object(WORKTREE_ROOT_KIND, ROOT_OBJECT_VERSION, &root)?;
        let target = IntentTarget {
            change_id: target_change,
            root_id: target_root.clone(),
            operation_id: None,
        };
        prepare_intent(
            &ChangedPathLedger::new(&fixture.db.conn),
            &fixture.expected,
            IntentProducer::Checkout,
            &target,
            &Fixture::evidence("new.rs"),
        )?;
        fixture.db.gc(false)?;
        let after_first: bool = fixture.db.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM objects WHERE object_id=?1)",
            [&target_root.0],
            |row| row.get(0),
        )?;
        if !after_first {
            return Err(Error::Corrupt(
                "GC collected an intent root at its recovery boundary".into(),
            ));
        }
        fixture.db.gc(false)?;
        let after_second: bool = fixture.db.conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM objects WHERE object_id=?1)",
            [&target_root.0],
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
                      NULL,NULL,NULL,'retire-segment.cplog','open',?2,NULL,?2)",
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
        if retired_paths != vec!["retire-segment.cplog"]
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
            mark_filesystem_applied(&ledger, &fixture.expected, &intent, &fixture.cut(9))?;
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
