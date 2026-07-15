use std::collections::BTreeSet;
use std::path::Path;
use std::time::Duration;

use getrandom::getrandom;
use rusqlite::{params, OptionalExtension, Transaction, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::{DirtyPrefix, EvidenceCut, EvidenceFlags, ExpectedScope, LedgerPath};
use crate::db::util::now_ts;
use crate::error::{Error, Result};
use crate::model::{OPERATION_KIND, WORKTREE_ROOT_KIND};
use crate::{ChangeId, ObjectId};

#[cfg(debug_assertions)]
thread_local! {
    static SIDECAR_ANCESTOR_SUBSTITUTION_HOOK: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(debug_assertions)]
pub(super) fn install_sidecar_ancestor_substitution_hook(hook: impl FnOnce() + 'static) {
    SIDECAR_ANCESTOR_SUBSTITUTION_HOOK.with(|slot| {
        *slot.borrow_mut() = Some(Box::new(hook));
    });
}

#[cfg(debug_assertions)]
fn run_sidecar_ancestor_substitution_hook() {
    SIDECAR_ANCESTOR_SUBSTITUTION_HOOK.with(|slot| {
        if let Some(hook) = slot.borrow_mut().take() {
            hook();
        }
    });
}

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
pub(crate) struct QualifiedFilesystemProof {
    pub(crate) scope_id: super::ScopeId,
    pub(crate) epoch: u64,
    pub(crate) expected_root_id: ObjectId,
    pub(crate) scope_root_identity: Vec<u8>,
    pub(crate) filesystem_identity: Vec<u8>,
    pub(crate) provider_id: String,
    pub(crate) provider_identity: Vec<u8>,
    pub(crate) observer_owner_token: String,
    pub(crate) owner_fence_nonce: Option<Vec<u8>>,
    pub(crate) durable_segment_id: String,
    pub(crate) durable_segment_hash: [u8; 32],
    pub(crate) segment_directory: String,
    pub(crate) segment_path: String,
    pub(crate) start_cursor: Option<Vec<u8>>,
    pub(crate) end_cursor: Vec<u8>,
    pub(crate) start_sequence: u64,
    /// Cut through which the controlled apply was pinned and may be
    /// acknowledged. Events after this cut are never consumed by publication.
    pub(crate) end_cut: EvidenceCut,
    /// Final folded cut used only for the scope baseline CAS. Evidence in
    /// `(end_cut, publication_cut]` remains pending for the next snapshot.
    pub(crate) publication_cut: EvidenceCut,
    pub(crate) segment_durable_offset: u64,
    pub(crate) segment_folded_offset: u64,
    pub(crate) verified_paths: u64,
    pub(crate) verified_prefixes: u64,
    pub(crate) complete_root_interval: bool,
    pub(crate) complete_policy_interval: bool,
    pub(crate) persisted_evidence_through_end: bool,
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
    pub(super) start_cursor: Option<Vec<u8>>,
    pub(super) verified_cut: Option<QualifiedFilesystemProof>,
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
    exact_scope_guard(&tx, expected, true)?;
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
            params![id.0, path.as_str(), EvidenceFlags::ANY_MUTATION.0],
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
                EvidenceFlags::ANY_MUTATION.0
            ],
        )?;
    }
    tx.commit()?;
    durable_intent_barrier(ledger.conn)?;
    crate::db::util::test_crash_point("changed_path_after_intent_prepare");
    Ok(id)
}

pub(crate) fn mark_filesystem_applied(
    ledger: &super::ChangedPathLedger<'_>,
    expected: &ExpectedScope,
    intent_id: &IntentId,
    proof: &QualifiedFilesystemProof,
) -> Result<()> {
    let tx = Transaction::new_unchecked(ledger.conn, TransactionBehavior::Immediate)?;
    exact_scope_guard(&tx, expected, true)?;
    let intent = load_intent(&tx, intent_id)?.ok_or_else(|| {
        Error::InvalidInput(format!("unknown changed-path intent `{}`", intent_id.0))
    })?;
    if intent.scope_id != expected.scope_id.to_text() || intent.state != IntentState::Prepared {
        return Err(Error::Conflict(format!(
            "intent `{}` is not in prepared state for this scope",
            intent_id.0
        )));
    }
    validate_qualified_filesystem_proof(&tx, ledger.database_path()?, expected, &intent, proof)?;
    let changed = tx.execute(
        "UPDATE changed_path_intents SET lifecycle_state='filesystem_applied',verified_cut=?1,
             updated_at=?2
         WHERE intent_id=?3 AND scope_id=?4 AND lifecycle_state='prepared'",
        params![
            serde_json::to_vec(proof)?,
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
    crate::db::util::test_crash_point("changed_path_after_filesystem_applied");
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
    crate::db::util::test_crash_point("changed_path_after_intent_publish");
    Ok(())
}

pub(super) fn load_intent(
    conn: &rusqlite::Connection,
    id: &IntentId,
) -> Result<Option<PersistedIntent>> {
    conn.query_row(
        "SELECT intent_id,scope_id,lifecycle_state,expected_scope_epoch,expected_ref_name,
                expected_ref_generation,expected_change_id,expected_root_id,target_change_id,
                target_root_id,target_operation_id,start_cursor,verified_cut
         FROM changed_path_intents WHERE intent_id=?1",
        [&id.0],
        |row| {
            let state = row.get::<_, String>(2)?;
            let verified = row.get::<_, Option<Vec<u8>>>(12)?;
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
                row.get::<_, Option<Vec<u8>>>(11)?,
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
            start_cursor: row.11,
            verified_cut: row
                .12
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
        .map(|proof| proof.end_cut.sequence)
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

pub(super) fn validate_qualified_filesystem_proof(
    conn: &rusqlite::Connection,
    database_path: &Path,
    expected: &ExpectedScope,
    intent: &PersistedIntent,
    proof: &QualifiedFilesystemProof,
) -> Result<()> {
    let invalid = |reason: &str| Error::ChangeLedgerReconcileRequired {
        scope: expected.scope_id.to_text(),
        state: "untrusted_gap".into(),
        reason: reason.into(),
        command: "trail status".into(),
    };
    if proof.scope_id != expected.scope_id
        || proof.epoch != expected.epoch
        || proof.expected_root_id != intent.expected_root_id
        || proof.filesystem_identity != expected.filesystem_identity
        || proof.provider_identity != expected.provider_identity
        || proof.start_cursor != intent.start_cursor
        || proof.end_cut.source != super::EvidenceSource::Observer
        || proof.publication_cut.source != super::EvidenceSource::Observer
        || proof.start_sequence > proof.end_cut.sequence
        || proof.end_cut.sequence > proof.publication_cut.sequence
        || proof.end_cut.durable_offset != proof.end_cut.folded_offset
        || proof.publication_cut.durable_offset != proof.publication_cut.folded_offset
        || proof.segment_folded_offset != proof.segment_durable_offset
        || proof.observer_owner_token.is_empty()
        || proof.provider_id.is_empty()
        || proof.durable_segment_id.is_empty()
        || proof.segment_directory.is_empty()
        || proof.segment_path.is_empty()
        || !proof.complete_root_interval
        || !proof.complete_policy_interval
        || !proof.persisted_evidence_through_end
    {
        return Err(invalid(
            "filesystem proof identity or completeness is invalid",
        ));
    }

    let scope = conn
        .query_row(
            "SELECT scope_root_identity,filesystem_identity,provider_id,provider_identity,
                    provider_cursor,durable_offset,folded_offset,clean_proof_allowed,
                    linearizable_fence,filesystem_supported,power_loss_durability,trust_state
             FROM changed_path_scopes WHERE scope_id=?1 AND epoch=?2",
            params![
                expected.scope_id.to_text(),
                sql_u64(expected.epoch, "scope epoch")?
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<Vec<u8>>>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, bool>(7)?,
                    row.get::<_, bool>(8)?,
                    row.get::<_, bool>(9)?,
                    row.get::<_, bool>(10)?,
                    row.get::<_, String>(11)?,
                ))
            },
        )
        .optional()?;
    let Some((
        scope_root_identity,
        filesystem_identity,
        provider_id,
        provider_identity,
        provider_cursor,
        durable_offset,
        folded_offset,
        clean_proof_allowed,
        linearizable_fence,
        filesystem_supported,
        power_loss_durability,
        trust_state,
    )) = scope
    else {
        return Err(invalid("filesystem proof scope disappeared"));
    };
    if trust_state != "trusted"
        || !clean_proof_allowed
        || !linearizable_fence
        || !filesystem_supported
        || !power_loss_durability
        || hex::decode(scope_root_identity).ok().as_deref()
            != Some(proof.scope_root_identity.as_slice())
        || hex::decode(filesystem_identity).ok().as_deref()
            != Some(proof.filesystem_identity.as_slice())
        || provider_id.as_deref() != Some(proof.provider_id.as_str())
        || provider_identity
            .as_deref()
            .and_then(|identity| hex::decode(identity).ok())
            .as_deref()
            != Some(proof.provider_identity.as_slice())
        || db_u64(durable_offset, "scope durable offset")? != proof.publication_cut.durable_offset
        || db_u64(folded_offset, "scope folded offset")? != proof.publication_cut.folded_offset
    {
        return Err(invalid(
            "filesystem proof no longer matches the trusted scope boundary",
        ));
    }

    let owner_matches: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM changed_path_observer_owners
         WHERE scope_id=?1 AND epoch=?2 AND owner_token=?3 AND provider_id=?4
           AND provider_identity=?5 AND fence_nonce IS ?6 AND lease_state='active'
           AND error_state IS NULL AND error_at IS NULL AND expires_at>?7)",
        params![
            expected.scope_id.to_text(),
            sql_u64(expected.epoch, "scope epoch")?,
            proof.observer_owner_token,
            proof.provider_id,
            hex::encode(&proof.provider_identity),
            proof.owner_fence_nonce,
            now_ts(),
        ],
        |row| row.get(0),
    )?;
    let segment_matches: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM changed_path_observer_segments
         WHERE scope_id=?1 AND epoch=?2 AND segment_id=?3 AND owner_token=?4
           AND provider_id=?5 AND first_sequence<=?6 AND last_sequence=?7
           AND durable_end_offset=?8 AND folded_end_offset=?9
           AND segment_hash=?10 AND segment_path=?11 AND state='sealed')",
        params![
            expected.scope_id.to_text(),
            sql_u64(expected.epoch, "scope epoch")?,
            proof.durable_segment_id,
            proof.observer_owner_token,
            proof.provider_id,
            sql_u64(proof.start_sequence, "proof start sequence")?,
            sql_u64(proof.end_cut.sequence, "proof end sequence")?,
            sql_u64(proof.segment_durable_offset, "segment durable offset")?,
            sql_u64(proof.segment_folded_offset, "segment folded offset")?,
            hex::encode(proof.durable_segment_hash),
            proof.segment_path,
        ],
        |row| row.get(0),
    )?;
    if !owner_matches || !segment_matches {
        return Err(invalid(
            "filesystem proof owner, fence, or sealed segment changed",
        ));
    }

    let expected_directory = format!("observer-segments/{}", expected.scope_id.to_text());
    if proof.segment_directory != expected_directory {
        return Err(invalid(
            "filesystem proof segment directory is not the confined scope directory",
        ));
    }
    let database_root = database_path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| invalid("filesystem proof database has no Trail root"))?;
    let trail_directory = super::secure_fs::SecureDirectory::open_absolute(database_root)
        .map_err(|error| invalid(&format!("open Trail root securely: {error}")))?;
    let observer_directory = trail_directory
        .open_dir("observer-segments")
        .map_err(|error| invalid(&format!("open observer root securely: {error}")))?;
    #[cfg(debug_assertions)]
    run_sidecar_ancestor_substitution_hook();
    let segment_directory = observer_directory
        .open_dir(&expected.scope_id.to_text())
        .map_err(|error| invalid(&format!("open observer scope securely: {error}")))?;
    let owner_token: [u8; 32] = hex::decode(&proof.observer_owner_token)
        .ok()
        .and_then(|bytes| bytes.try_into().ok())
        .ok_or_else(|| invalid("filesystem proof owner token is not canonical"))?;
    let raw_limits: (i64, i64, i64) = conn.query_row(
        "SELECT max_observer_log_bytes,max_segment_bytes,max_unfolded_tail_records
         FROM changed_path_scopes WHERE scope_id=?1 AND epoch=?2",
        params![
            expected.scope_id.to_text(),
            sql_u64(expected.epoch, "scope epoch")?
        ],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    )?;
    let limits = super::PersistedLogLimits {
        max_log_bytes: db_u64(raw_limits.0, "observer log byte limit")?,
        max_segment_bytes: db_u64(raw_limits.1, "segment byte limit")?,
        max_unfolded_tail_records: usize::try_from(db_u64(
            raw_limits.2,
            "unfolded observer record limit",
        )?)
        .map_err(|_| Error::Corrupt("unfolded observer record limit exceeds usize".into()))?,
    };
    let recovery_scope = super::RecoveryScope {
        scope_id: expected.scope_id,
        epoch: expected.epoch,
        owner_token,
    };
    let mut recovered = None;
    for attempt in 0..16 {
        let candidate = super::recover_segments_from_directory(
            database_path,
            &segment_directory,
            &recovery_scope,
            limits,
        )
        .map_err(|error| invalid(&format!("observer sidecar verification failed: {error}")))?;
        if !candidate.requires_reconciliation || attempt == 15 {
            recovered = Some(candidate);
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let recovered = recovered.ok_or_else(|| invalid("observer sidecar verification vanished"))?;
    if recovered.requires_reconciliation {
        return Err(invalid(
            "observer sidecar chain is incomplete or contains unpublished entries",
        ));
    }
    let authenticated = recovered
        .segments
        .iter()
        .find(|segment| segment.segment_id == proof.durable_segment_id)
        .ok_or_else(|| invalid("filesystem proof segment is absent from verified chain"))?;
    let publication_boundary = recovered.record_boundaries.iter().find(|boundary| {
        boundary.sequence == proof.publication_cut.sequence
            && boundary.durable_end_offset == proof.publication_cut.durable_offset
            && provider_cursor.as_deref() == Some(boundary.provider_cursor.as_slice())
    });
    let publication_authenticated = publication_boundary.is_some_and(|boundary| {
        recovered.segments.iter().any(|segment| {
            segment.segment_id == boundary.segment_id
                && matches!(segment.state.as_str(), "open" | "sealed")
                && segment.first_sequence <= boundary.sequence
                && segment.last_sequence >= boundary.sequence
                && segment.durable_end_offset >= boundary.durable_end_offset
                && segment.folded_end_offset == proof.publication_cut.folded_offset
        })
    });
    if authenticated.state != "sealed"
        || authenticated.segment_path != proof.segment_path
        || authenticated.start_cursor != proof.start_cursor.clone().unwrap_or_default()
        || authenticated.end_cursor != proof.end_cursor
        || authenticated.first_sequence != proof.start_sequence
        || authenticated.last_sequence != proof.end_cut.sequence
        || authenticated.durable_end_offset != proof.segment_durable_offset
        || authenticated.folded_end_offset < proof.segment_folded_offset
        || authenticated.segment_hash != proof.durable_segment_hash
        || proof.end_cut.durable_offset != proof.segment_durable_offset
        || proof.end_cut.folded_offset != proof.segment_folded_offset
        || recovered.last_sequence < proof.publication_cut.sequence
        || !publication_authenticated
    {
        return Err(invalid(
            "filesystem proof is not an authenticated prefix of the observer chain",
        ));
    }

    let intent_paths = conn
        .prepare(
            "SELECT normalized_path,event_flags FROM changed_path_intent_paths
             WHERE intent_id=?1 ORDER BY normalized_path COLLATE BINARY",
        )?
        .query_map([&intent.id.0], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let intent_prefixes = conn
        .prepare(
            "SELECT normalized_prefix,completeness_reason,event_flags
             FROM changed_path_intent_prefixes
             WHERE intent_id=?1 ORDER BY normalized_prefix COLLATE BINARY",
        )?
        .query_map([&intent.id.0], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    let mut exact_flags = std::collections::BTreeMap::<&str, i64>::new();
    let mut prefix_flags = std::collections::BTreeMap::<&str, i64>::new();
    for record in recovered.records.iter().filter(|record| {
        record.source == super::EvidenceSource::Observer
            && record.sequence >= proof.start_sequence
            && record.sequence <= proof.end_cut.sequence
    }) {
        let flags = record.flags.0 & !super::EvidenceFlags::PROVIDER_COMPLETE_PREFIX.0;
        *exact_flags.entry(record.path.as_str()).or_default() |= flags;
        if record.flags.0 & super::EvidenceFlags::PROVIDER_COMPLETE_PREFIX.0 != 0 {
            *prefix_flags.entry(record.path.as_str()).or_default() |= flags;
        }
    }
    let paths_covered = intent_paths.iter().all(|(path, required)| {
        exact_flags
            .get(path.as_str())
            .is_some_and(|observed| observed & required != 0)
    });
    let prefixes_covered = intent_prefixes.iter().all(|(prefix, reason, required)| {
        reason == "provider_complete"
            && prefix_flags
                .get(prefix.as_str())
                .is_some_and(|observed| observed & required != 0)
    });
    if !paths_covered
        || !prefixes_covered
        || u64::try_from(intent_paths.len()).ok() != Some(proof.verified_paths)
        || u64::try_from(intent_prefixes.len()).ok() != Some(proof.verified_prefixes)
    {
        return Err(invalid(
            "filesystem proof interval does not contain all authenticated intent evidence",
        ));
    }
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
