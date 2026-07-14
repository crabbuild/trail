use std::collections::BTreeSet;

use getrandom::getrandom;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::{DirtyPrefix, EvidenceCut, EvidenceFlags, ExpectedScope, LedgerPath};
use crate::db::util::now_ts;
use crate::error::{Error, Result};
use crate::model::{OPERATION_KIND, WORKTREE_ROOT_KIND};
use crate::{ChangeId, ObjectId};

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct IntentId(pub(crate) String);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntentProducer {
    Checkout,
    LaneSync,
    Materialize,
    StructuredPatchProjection,
    RestoreProjection,
    CowPublication,
    ObservedCheckpoint,
}

impl IntentProducer {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Checkout => "checkout",
            Self::LaneSync => "lane_sync",
            Self::Materialize => "materialize",
            Self::StructuredPatchProjection => "structured_patch_projection",
            Self::RestoreProjection => "restore_projection",
            Self::CowPublication => "cow_publication",
            Self::ObservedCheckpoint => "observed_checkpoint",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum IntentState {
    Prepared,
    FilesystemApplied,
    Published,
    Acknowledged,
    Aborted,
}

impl IntentState {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Prepared => "prepared",
            Self::FilesystemApplied => "filesystem_applied",
            Self::Published => "published",
            Self::Acknowledged => "acknowledged",
            Self::Aborted => "aborted",
        }
    }

    pub(super) fn parse(value: &str) -> Result<Self> {
        match value {
            "prepared" => Ok(Self::Prepared),
            "filesystem_applied" => Ok(Self::FilesystemApplied),
            "published" => Ok(Self::Published),
            "acknowledged" => Ok(Self::Acknowledged),
            "aborted" => Ok(Self::Aborted),
            other => Err(Error::Corrupt(format!(
                "unknown changed-path intent state `{other}`"
            ))),
        }
    }

    pub(super) const fn is_terminal(self) -> bool {
        matches!(self, Self::Acknowledged | Self::Aborted)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IntentTarget {
    pub(crate) change_id: ChangeId,
    pub(crate) root_id: ObjectId,
    pub(crate) operation_id: Option<ObjectId>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct IntentEvidence {
    pub(crate) exact_paths: Vec<LedgerPath>,
    pub(crate) complete_prefixes: Vec<DirtyPrefix>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct VerifiedFilesystemCut {
    pub(crate) observer_cut: EvidenceCut,
    pub(crate) verified_paths: u64,
    pub(crate) filesystem_identity: Vec<u8>,
}

#[derive(Clone, Debug)]
pub(super) struct PersistedIntent {
    pub(super) id: IntentId,
    pub(super) scope_id: String,
    pub(super) state: IntentState,
    pub(super) expected_epoch: u64,
    pub(super) expected_ref_name: String,
    pub(super) expected_ref_generation: u64,
    pub(super) expected_change_id: ChangeId,
    pub(super) expected_root_id: ObjectId,
    pub(super) target: IntentTarget,
    pub(super) verified_cut: Option<VerifiedFilesystemCut>,
}

pub(crate) fn prepare_intent(
    ledger: &super::ChangedPathLedger<'_>,
    expected: &ExpectedScope,
    producer: IntentProducer,
    target: &IntentTarget,
    evidence: &IntentEvidence,
) -> Result<IntentId> {
    validate_evidence(evidence)?;
    ledger.recover_scope(expected)?;
    let tx = Transaction::new_unchecked(ledger.conn, TransactionBehavior::Immediate)?;
    exact_scope_guard(&tx, expected, false)?;
    let pending: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM changed_path_intents
         WHERE scope_id=?1 AND lifecycle_state IN ('prepared','filesystem_applied','published'))",
        [expected.scope_id.to_text()],
        |row| row.get(0),
    )?;
    if pending {
        return Err(Error::Conflict(
            "changed-path scope still has a nonterminal intent".into(),
        ));
    }
    let (expected_change_id, start_cursor) = tx.query_row(
        "SELECT change_id,provider_cursor FROM changed_path_scopes WHERE scope_id=?1",
        [expected.scope_id.to_text()],
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<Vec<u8>>>(1)?)),
    )?;
    let id = IntentId(new_intent_id()?);
    let now = now_ts();
    tx.execute(
        "INSERT INTO changed_path_intents(
             intent_id,schema_version,scope_id,producer,expected_scope_epoch,
             expected_ref_name,expected_ref_generation,expected_change_id,expected_root_id,
             target_change_id,target_root_id,target_operation_id,start_cursor,lifecycle_state,
             verified_cut,failure_reason,created_at,updated_at
         ) VALUES(?1,1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,'prepared',NULL,NULL,?13,?13)",
        params![
            id.0,
            expected.scope_id.to_text(),
            producer.as_str(),
            sql_u64(expected.epoch, "scope epoch")?,
            expected.ref_name,
            sql_u64(expected.ref_generation, "ref generation")?,
            expected_change_id,
            expected.baseline_root.0,
            target.change_id.0,
            target.root_id.0,
            target.operation_id.as_ref().map(|id| id.0.as_str()),
            start_cursor,
            now,
        ],
    )?;
    for path in &evidence.exact_paths {
        tx.execute(
            "INSERT INTO changed_path_intent_paths(intent_id,normalized_path,event_flags)
             VALUES(?1,?2,?3)",
            params![id.0, path.as_str(), EvidenceFlags::CONTENT.0],
        )?;
    }
    for prefix in &evidence.complete_prefixes {
        tx.execute(
            "INSERT INTO changed_path_intent_prefixes(
                 intent_id,normalized_prefix,completeness_reason,event_flags
             ) VALUES(?1,?2,?3,?4)",
            params![
                id.0,
                prefix.path.as_str(),
                prefix.reason,
                EvidenceFlags::CONTENT.0
            ],
        )?;
    }
    tx.commit()?;
    durable_intent_barrier(ledger.conn)?;
    Ok(id)
}

pub(crate) fn mark_filesystem_applied(
    ledger: &super::ChangedPathLedger<'_>,
    expected: &ExpectedScope,
    intent_id: &IntentId,
    cut: &VerifiedFilesystemCut,
) -> Result<()> {
    if cut.observer_cut.source != super::EvidenceSource::Observer {
        return Err(Error::InvalidInput(
            "verified filesystem cut must be fenced by the observer".into(),
        ));
    }
    if cut.filesystem_identity != expected.filesystem_identity {
        return Err(Error::InvalidInput(
            "verified filesystem cut belongs to another filesystem".into(),
        ));
    }
    if cut.observer_cut.durable_offset != cut.observer_cut.folded_offset {
        return Err(Error::InvalidInput(
            "verified filesystem cut has an unfolded observer tail".into(),
        ));
    }
    let tx = Transaction::new_unchecked(ledger.conn, TransactionBehavior::Immediate)?;
    exact_scope_guard(&tx, expected, false)?;
    let (durable_offset, folded_offset) = tx.query_row(
        "SELECT durable_offset,folded_offset FROM changed_path_scopes WHERE scope_id=?1",
        [expected.scope_id.to_text()],
        |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
    )?;
    if db_u64(durable_offset, "scope durable offset")? != cut.observer_cut.durable_offset
        || db_u64(folded_offset, "scope folded offset")? != cut.observer_cut.folded_offset
    {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "verified filesystem cut does not match persisted observer boundaries".into(),
            command: "trail status".into(),
        });
    }
    let changed = tx.execute(
        "UPDATE changed_path_intents SET lifecycle_state='filesystem_applied',verified_cut=?1,
             updated_at=?2
         WHERE intent_id=?3 AND scope_id=?4 AND lifecycle_state='prepared'",
        params![
            serde_json::to_vec(cut)?,
            now_ts(),
            intent_id.0,
            expected.scope_id.to_text()
        ],
    )?;
    if changed != 1 {
        return Err(Error::Conflict(format!(
            "intent `{}` is not in prepared state",
            intent_id.0
        )));
    }
    tx.commit()?;
    durable_intent_barrier(ledger.conn)?;
    Ok(())
}

pub(crate) fn publish_intent(
    ledger: &super::ChangedPathLedger<'_>,
    expected: &ExpectedScope,
    intent_id: &IntentId,
) -> Result<()> {
    let tx = Transaction::new_unchecked(ledger.conn, TransactionBehavior::Immediate)?;
    exact_scope_guard(&tx, expected, true)?;
    let intent = load_intent(&tx, intent_id)?.ok_or_else(|| {
        Error::InvalidInput(format!("unknown changed-path intent `{}`", intent_id.0))
    })?;
    if intent.scope_id != expected.scope_id.to_text()
        || intent.state != IntentState::FilesystemApplied
    {
        return Err(Error::Conflict(format!(
            "intent `{}` is not filesystem-applied for this scope",
            intent_id.0
        )));
    }
    if !authoritative_ref_matches_target(&tx, &intent)? {
        return Err(Error::StaleBranch(intent.expected_ref_name));
    }
    stage_intent_evidence(&tx, &intent)?;
    let changed = tx.execute(
        "UPDATE changed_path_intents SET lifecycle_state='published',updated_at=?1
         WHERE intent_id=?2 AND lifecycle_state='filesystem_applied'",
        params![now_ts(), intent_id.0],
    )?;
    if changed != 1 {
        return Err(Error::Conflict(
            "intent publication raced another transition".into(),
        ));
    }
    tx.commit()?;
    durable_intent_barrier(ledger.conn)?;
    Ok(())
}

pub(super) fn load_intent(
    conn: &rusqlite::Connection,
    id: &IntentId,
) -> Result<Option<PersistedIntent>> {
    conn.query_row(
        "SELECT intent_id,scope_id,lifecycle_state,expected_scope_epoch,expected_ref_name,
                expected_ref_generation,expected_change_id,expected_root_id,target_change_id,
                target_root_id,target_operation_id,verified_cut
         FROM changed_path_intents WHERE intent_id=?1",
        [&id.0],
        |row| {
            let state = row.get::<_, String>(2)?;
            let verified = row.get::<_, Option<Vec<u8>>>(11)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                state,
                row.get::<_, i64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, i64>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, Option<String>>(10)?,
                verified,
            ))
        },
    )
    .optional()?
    .map(|row| {
        Ok(PersistedIntent {
            id: IntentId(row.0),
            scope_id: row.1,
            state: IntentState::parse(&row.2)?,
            expected_epoch: db_u64(row.3, "intent epoch")?,
            expected_ref_name: row.4,
            expected_ref_generation: db_u64(row.5, "intent ref generation")?,
            expected_change_id: ChangeId(row.6),
            expected_root_id: ObjectId(row.7),
            target: IntentTarget {
                change_id: ChangeId(row.8),
                root_id: ObjectId(row.9),
                operation_id: row.10.map(ObjectId),
            },
            verified_cut: row
                .11
                .map(|bytes| serde_json::from_slice(&bytes))
                .transpose()?,
        })
    })
    .transpose()
}

pub(super) fn stage_intent_evidence(tx: &Transaction<'_>, intent: &PersistedIntent) -> Result<()> {
    let sequence = intent
        .verified_cut
        .as_ref()
        .map(|cut| cut.observer_cut.sequence)
        .unwrap_or_default();
    let sequence = sql_u64(sequence, "intent sequence")?;
    let now = now_ts();
    tx.execute(
        "INSERT INTO changed_path_entries(scope_id,normalized_path,event_flags,source_mask,
             first_sequence,last_sequence,provider_id,provider_sequence,intent_id,created_at,updated_at)
         SELECT i.scope_id,p.normalized_path,p.event_flags,2,?1,?1,'intent',NULL,i.intent_id,?2,?2
         FROM changed_path_intent_paths p JOIN changed_path_intents i ON i.intent_id=p.intent_id
         WHERE p.intent_id=?3
         ON CONFLICT(scope_id,normalized_path) DO UPDATE SET
             event_flags=changed_path_entries.event_flags|excluded.event_flags,
             source_mask=changed_path_entries.source_mask|excluded.source_mask,
             first_sequence=MIN(changed_path_entries.first_sequence,excluded.first_sequence),
             last_sequence=MAX(changed_path_entries.last_sequence,excluded.last_sequence),
             intent_id=excluded.intent_id,updated_at=excluded.updated_at",
        params![sequence, now, intent.id.0],
    )?;
    tx.execute(
        "INSERT INTO changed_path_prefixes(scope_id,normalized_prefix,completeness_reason,
             event_flags,source_mask,first_sequence,last_sequence,provider_id,provider_sequence,
             intent_id,created_at,updated_at)
         SELECT i.scope_id,p.normalized_prefix,p.completeness_reason,p.event_flags,2,?1,?1,
                'intent',NULL,i.intent_id,?2,?2
         FROM changed_path_intent_prefixes p JOIN changed_path_intents i ON i.intent_id=p.intent_id
         WHERE p.intent_id=?3
         ON CONFLICT(scope_id,normalized_prefix) DO UPDATE SET
             event_flags=changed_path_prefixes.event_flags|excluded.event_flags,
             source_mask=changed_path_prefixes.source_mask|excluded.source_mask,
             first_sequence=MIN(changed_path_prefixes.first_sequence,excluded.first_sequence),
             last_sequence=MAX(changed_path_prefixes.last_sequence,excluded.last_sequence),
             intent_id=excluded.intent_id,updated_at=excluded.updated_at",
        params![sequence, now, intent.id.0],
    )?;
    Ok(())
}

pub(super) fn authoritative_ref_matches_target(
    conn: &rusqlite::Connection,
    intent: &PersistedIntent,
) -> Result<bool> {
    let observed = conn
        .query_row(
            "SELECT change_id,root_id,operation_id,generation FROM refs WHERE name=?1",
            [&intent.expected_ref_name],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                ))
            },
        )
        .optional()?;
    let Some((change, root, operation, generation)) = observed else {
        return Ok(false);
    };
    let root_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM objects WHERE object_id=?1 AND kind=?2)",
        params![intent.target.root_id.0, WORKTREE_ROOT_KIND],
        |row| row.get(0),
    )?;
    let operation_exists = match &intent.target.operation_id {
        Some(operation_id) => conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM objects o JOIN operations p ON p.operation_id=o.object_id
                 WHERE o.object_id=?1 AND o.kind=?2 AND p.change_id=?3 AND p.after_root=?4
             )",
            params![
                operation_id.0,
                OPERATION_KIND,
                intent.target.change_id.0,
                intent.target.root_id.0
            ],
            |row| row.get(0),
        )?,
        None => true,
    };
    Ok(root_exists
        && operation_exists
        && change == intent.target.change_id.0
        && root == intent.target.root_id.0
        && intent
            .target
            .operation_id
            .as_ref()
            .is_none_or(|id| id.0 == operation)
        && db_u64(generation, "authoritative ref generation")?
            == intent.expected_ref_generation.saturating_add(1))
}

pub(super) fn exact_scope_guard(
    conn: &rusqlite::Connection,
    expected: &ExpectedScope,
    trusted: bool,
) -> Result<()> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM changed_path_scopes WHERE scope_id=?1 AND epoch=?2
         AND ref_name=?3 AND ref_generation=?4 AND baseline_root_id=?5
         AND policy_fingerprint=?6 AND policy_dependency_generation=?7
         AND filesystem_identity=?8 AND provider_identity=?9
         AND (?10=0 OR trust_state='trusted'))",
        params![
            expected.scope_id.to_text(),
            sql_u64(expected.epoch, "scope epoch")?,
            expected.ref_name,
            sql_u64(expected.ref_generation, "ref generation")?,
            expected.baseline_root.0,
            hex::encode(expected.policy_fingerprint),
            sql_u64(expected.policy_generation, "policy generation")?,
            hex::encode(&expected.filesystem_identity),
            hex::encode(&expected.provider_identity),
            i64::from(trusted)
        ],
        |row| row.get(0),
    )?;
    if !exists {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "stale intent scope CAS".into(),
            command: "trail status".into(),
        });
    }
    Ok(())
}

fn validate_evidence(evidence: &IntentEvidence) -> Result<()> {
    let mut paths = BTreeSet::new();
    for path in &evidence.exact_paths {
        if !paths.insert(path.as_str()) {
            return Err(Error::InvalidInput("duplicate intent path".into()));
        }
    }
    let mut prefixes = BTreeSet::new();
    for prefix in &evidence.complete_prefixes {
        if !prefix.complete || prefix.reason.is_empty() {
            return Err(Error::InvalidInput(
                "intent prefixes require complete nonempty evidence".into(),
            ));
        }
        if !prefixes.insert(prefix.path.as_str()) {
            return Err(Error::InvalidInput("duplicate intent prefix".into()));
        }
    }
    Ok(())
}

fn new_intent_id() -> Result<String> {
    let mut bytes = [0_u8; 32];
    getrandom(&mut bytes).map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
    Ok(format!("intent-{}", hex::encode(bytes)))
}

pub(super) fn sql_u64(value: u64, label: &str) -> Result<i64> {
    value
        .try_into()
        .map_err(|_| Error::InvalidInput(format!("{label} exceeds SQLite INTEGER range")))
}

pub(super) fn db_u64(value: i64, label: &str) -> Result<u64> {
    value
        .try_into()
        .map_err(|_| Error::Corrupt(format!("{label} is negative")))
}

pub(super) fn durable_intent_barrier(conn: &rusqlite::Connection) -> Result<()> {
    let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(FULL)", [], |row| row.get(0))?;
    if busy != 0 {
        return Err(Error::Conflict(
            "changed-path intent durability checkpoint remained busy".into(),
        ));
    }
    Ok(())
}
