//! Shared publication state machines for controlled filesystem producers.
//!
//! Applying filesystem bytes and proving their observer cut happens before
//! these functions are entered.  Publication is deliberately one SQLite
//! transaction: an indexed operation, ref generation, lane head, ledger
//! baseline, intent terminal state, and acknowledgement can never become
//! visible at different cuts.  Ref and marker mirrors are repaired only after
//! commit by the caller.

use rusqlite::{params, Transaction, TransactionBehavior};
use std::collections::BTreeSet;
use std::time::Duration;

use super::intent::{
    authoritative_ref_matches_target, exact_scope_guard, load_intent, stage_intent_evidence,
    IntentEvidence, IntentId, IntentProducer, IntentState, IntentTarget,
};
use super::types::{trail_atomic_temp_target, trail_case_probe_token};
use super::{
    BaselineIdentity, EvidenceSource, ExpectedScope, ObservedRecordCut, QualifiedFilesystemProof,
};
use super::{EvidenceAcknowledgementToken, EvidenceFlags, EvidenceRowKind, LedgerPath};
use crate::db::util::now_ts;
use crate::error::{Error, Result};
use crate::model::{Operation, RefRecord};
use crate::{ChangeId, ObjectId};

#[derive(Clone, Debug)]
pub(crate) struct ProjectionPublication {
    pub(crate) operation_id: ObjectId,
    pub(crate) baseline: BaselineIdentity,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceViewCheckpointPublication {
    pub(crate) view_id: String,
    pub(crate) expected_generation: u64,
    pub(crate) journal_sequence: u64,
    pub(crate) next_generation: u64,
    pub(crate) journal_qualified: bool,
}

fn acquire_projection_publication_lock(db: &crate::Trail) -> Result<crate::db::WorkspaceLock> {
    // The exclusion is activated by every caller immediately before this
    // handoff. New observer durability writers are therefore blocked; a
    // writer that acquired the lock just before exclusion gets a short,
    // bounded interval to finish and release it. External or stuck ownership
    // still fails closed at the deadline.
    crate::db::acquire_workspace_lock_with_admission(
        &db.db_dir,
        &db.db_dir.join(crate::db::DB_RELATIVE_PATH),
        crate::db::WorkspaceLockAdmission {
            purpose: crate::db::WorkspaceLockPurpose::ObserverPublication,
            operation_id: None,
            deadline: Duration::from_secs(2),
            retry_command: "retry the command",
        },
    )
}

pub(crate) enum ProjectionAlignmentMode {
    Aligned,
    RetainDirty { target: IntentTarget },
}

pub(crate) enum RefAdvancingProjectionMode<'a> {
    ControlledIntent,
    UnalignedReconcileRequired {
        reason: &'a str,
    },
    LayeredUpdateReconcileRequired {
        reason: &'a str,
        lane_base_change: &'a ChangeId,
        lane_base_root: &'a ObjectId,
        view_id: &'a str,
        checkpoint_sequence: u64,
    },
    ObservedCut {
        cut: &'a ObservedRecordCut,
        acknowledge_complete_prefixes: bool,
    },
}

/// Complete projection-only protocol: recover and prepare against the exact
/// unchanged ref/root, apply and durably verify bytes, qualify the final
/// observer cut, publish terminal intent state atomically, then repair the
/// compact marker mirror. The apply closure must include filesystem sync and
/// pinned verification before returning its proof.
pub(crate) fn run_projection_alignment<A, M>(
    db: &mut crate::Trail,
    expected: &ExpectedScope,
    producer: IntentProducer,
    evidence: &IntentEvidence,
    mode: ProjectionAlignmentMode,
    apply_sync_verify_and_fence: A,
    repair_marker_after_commit: M,
) -> Result<IntentId>
where
    A: FnOnce(&mut crate::Trail, &IntentId) -> Result<QualifiedFilesystemProof>,
    M: FnOnce(&crate::Trail) -> Result<()>,
{
    if !matches!(
        producer,
        IntentProducer::Checkout | IntentProducer::LaneSync | IntentProducer::Materialize
    ) {
        return Err(Error::InvalidInput(
            "ref-advancing producer cannot use projection alignment".into(),
        ));
    }
    db.changed_path_ledger().recover_scope(expected)?;
    let current: (String, ObjectId) = db.conn.query_row(
        "SELECT change_id,root_id FROM refs WHERE name=?1 AND generation=?2",
        params![
            expected.ref_name,
            sql_u64(expected.ref_generation, "ref generation")?
        ],
        |row| Ok((row.get(0)?, ObjectId(row.get(1)?))),
    )?;
    if current.1 != expected.baseline_root {
        return Err(Error::StaleBranch(expected.ref_name.clone()));
    }
    let target = match mode {
        ProjectionAlignmentMode::Aligned => IntentTarget {
            change_id: crate::ChangeId(current.0),
            root_id: current.1,
            operation_id: None,
        },
        ProjectionAlignmentMode::RetainDirty { target } => target,
    };
    let retain_dirty = target.root_id != expected.baseline_root;
    let intent = super::prepare_intent(
        &db.changed_path_ledger(),
        expected,
        producer,
        &target,
        evidence,
    )?;
    let proof = apply_sync_verify_and_fence(db, &intent)?;
    let _observer_exclusion = crate::db::begin_authorized_observer_write_exclusion(&db.db_dir);
    let _write_lock = acquire_projection_publication_lock(db)?;
    super::mark_filesystem_applied(&db.changed_path_ledger(), expected, &intent, &proof)?;
    if retain_dirty {
        publish_dirty_alignment_in_transaction(db, expected, &intent)?;
    } else {
        publish_alignment_in_transaction(db, expected, &intent)?;
    }
    repair_marker_after_commit(db).map_err(|error| Error::OperationCommittedRepairRequired {
        operation: intent.0.clone(),
        repair: "projection marker".into(),
        reason: error.to_string(),
    })?;
    Ok(intent)
}

fn publish_dirty_alignment_in_transaction(
    db: &crate::Trail,
    expected: &ExpectedScope,
    intent_id: &IntentId,
) -> Result<()> {
    let conn = &db.conn;
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    exact_scope_guard(&tx, expected, true)?;
    let intent = load_intent(&tx, intent_id)?.ok_or_else(|| {
        Error::InvalidInput(format!("unknown changed-path intent `{}`", intent_id.0))
    })?;
    if intent.scope_id != expected.scope_id.to_text()
        || intent.state != IntentState::FilesystemApplied
        || intent.target.operation_id.is_some()
        || intent.target.root_id == expected.baseline_root
    {
        return Err(Error::Conflict(format!(
            "intent `{}` is not a dirty projection",
            intent_id.0
        )));
    }
    let proof = intent
        .verified_cut
        .as_ref()
        .ok_or_else(|| Error::Conflict(format!("intent `{}` has no qualified cut", intent_id.0)))?;
    super::intent::validate_qualified_filesystem_proof(
        &tx,
        ledger_database_path(&tx)?,
        expected,
        &intent,
        proof,
    )?;
    acknowledge_controlled_observer_evidence_in_transaction(db, &tx, expected, &intent, proof)?;
    // Restage intent evidence after removing only the verified controlled
    // observer rows. These rows deliberately remain pending against the
    // unchanged baseline, making checkout-to-another-root dirty.
    stage_intent_evidence(&tx, &intent)?;
    terminal_intent(&tx, intent_id)?;
    tx.commit()?;
    super::intent::durable_intent_barrier(conn)
}

/// Complete ref-advancing protocol: prebuild immutable operation/object,
/// recover and durably prepare, apply+sync+verify under a qualified observer
/// interval, and atomically publish operation/ref/lane/ledger/ack/terminal
/// state. Ref and marker mirrors are repaired only after SQLite commit.
pub(crate) fn run_ref_advancing_projection<A, M>(
    db: &mut crate::Trail,
    expected: &ExpectedScope,
    expected_ref: &RefRecord,
    lane_id: &str,
    producer: IntentProducer,
    operation: &Operation,
    evidence: &IntentEvidence,
    mode: RefAdvancingProjectionMode<'_>,
    apply_sync_verify_and_fence: A,
    repair_marker_after_commit: M,
) -> Result<ProjectionPublication>
where
    A: FnOnce(&mut crate::Trail, &IntentId) -> Result<QualifiedFilesystemProof>,
    M: FnOnce(&crate::Trail, &ProjectionPublication) -> Result<()>,
{
    if matches!(
        mode,
        RefAdvancingProjectionMode::UnalignedReconcileRequired { .. }
            | RefAdvancingProjectionMode::LayeredUpdateReconcileRequired { .. }
    ) {
        let reason = match &mode {
            RefAdvancingProjectionMode::UnalignedReconcileRequired { reason }
            | RefAdvancingProjectionMode::LayeredUpdateReconcileRequired { reason, .. } => *reason,
            _ => unreachable!(),
        };
        let _observer_exclusion = crate::db::begin_authorized_observer_write_exclusion(&db.db_dir);
        let _write_lock = acquire_projection_publication_lock(db)?;
        if operation.before_root.as_ref() != Some(&expected_ref.root_id)
            || operation.parents.first() != Some(&expected_ref.change_id)
            || operation.branch != expected_ref.name
        {
            return Err(Error::StaleBranch(expected_ref.name.clone()));
        }
        let (operation, operation_id) = db.store_operation_object_unindexed(operation)?;
        let tx = Transaction::new_unchecked(&db.conn, TransactionBehavior::Immediate)?;
        db.index_operation_in_transaction(&operation, &operation_id)?;
        db.advance_ref_cas_in_transaction(
            expected_ref,
            &operation.change_id,
            &operation.after_root,
            &operation_id,
        )?;
        let lane_changed = tx.execute(
            "UPDATE lane_branches SET head_change=?1,head_root=?2,updated_at=?3
             WHERE lane_id=?4 AND ref_name=?5 AND head_change=?6 AND head_root=?7",
            params![
                operation.change_id.0,
                operation.after_root.0,
                now_ts(),
                lane_id,
                expected_ref.name,
                expected_ref.change_id.0,
                expected_ref.root_id.0,
            ],
        )?;
        if lane_changed != 1 {
            return Err(Error::StaleBranch(expected_ref.name.clone()));
        }
        if let RefAdvancingProjectionMode::LayeredUpdateReconcileRequired {
            lane_base_change,
            lane_base_root,
            view_id,
            checkpoint_sequence,
            ..
        } = &mode
        {
            let base_changed = tx.execute(
                "UPDATE lane_branches
                 SET base_change=?1,base_root=?2,status='active',updated_at=?3
                 WHERE lane_id=?4 AND head_change=?5 AND head_root=?6",
                params![
                    lane_base_change.0,
                    lane_base_root.0,
                    now_ts(),
                    lane_id,
                    operation.change_id.0,
                    operation.after_root.0,
                ],
            )?;
            let view_changed = tx.execute(
                "UPDATE workspace_views
                 SET base_change=?1,base_root=?2,generation=generation+1,
                     checkpoint_seq=?3,checkpoint_root=?2,updated_at=?4
                 WHERE view_id=?5",
                params![
                    operation.change_id.0,
                    operation.after_root.0,
                    i64::try_from(*checkpoint_sequence).map_err(|_| {
                        Error::InvalidInput("workspace checkpoint sequence overflow".into())
                    })?,
                    now_ts(),
                    view_id,
                ],
            )?;
            if base_changed != 1 || view_changed != 1 {
                return Err(Error::StaleBranch(expected_ref.name.clone()));
            }
        }
        tx.execute(
            "UPDATE changed_path_scopes
             SET trust_state='stale_baseline',trust_reason=?1,updated_at=?2
             WHERE scope_id=?3 AND retired_at IS NULL",
            params![reason, now_ts(), expected.scope_id.to_text()],
        )?;
        tx.commit()?;
        db.repair_ref_mirror(
            expected_ref,
            &operation.change_id,
            &operation.after_root,
            &operation_id,
        )
        .map_err(|error| Error::OperationCommittedRepairRequired {
            operation: operation_id.0.clone(),
            repair: "ref mirror".into(),
            reason: error.to_string(),
        })?;
        let publication = ProjectionPublication {
            operation_id,
            baseline: BaselineIdentity {
                ref_name: expected_ref.name.clone(),
                ref_generation: u64::try_from(expected_ref.generation + 1)
                    .map_err(|_| Error::InvalidInput("ref generation overflow".into()))?,
                change_id: operation.change_id.clone(),
                root_id: operation.after_root.clone(),
            },
        };
        repair_marker_after_commit(db, &publication).map_err(|error| {
            Error::OperationCommittedRepairRequired {
                operation: publication.operation_id.0.clone(),
                repair: "projection marker/runtime".into(),
                reason: error.to_string(),
            }
        })?;
        return Ok(publication);
    }
    if let RefAdvancingProjectionMode::ObservedCut {
        cut,
        acknowledge_complete_prefixes,
    } = &mode
    {
        if producer != IntentProducer::ObservedCheckpoint || cut.expected != *expected {
            return Err(Error::InvalidInput(
                "observed ref projection requires its exact fenced scope".into(),
            ));
        }
        let _observer_exclusion = crate::db::begin_authorized_observer_write_exclusion(&db.db_dir);
        let _write_lock = acquire_projection_publication_lock(db)?;
        let operation_id = db.commit_observed_record(
            operation,
            expected_ref,
            cut,
            *acknowledge_complete_prefixes,
            Some(lane_id),
        )?;
        let publication = ProjectionPublication {
            operation_id,
            baseline: BaselineIdentity {
                ref_name: expected_ref.name.clone(),
                ref_generation: u64::try_from(expected_ref.generation + 1)
                    .map_err(|_| Error::InvalidInput("ref generation overflow".into()))?,
                change_id: operation.change_id.clone(),
                root_id: operation.after_root.clone(),
            },
        };
        repair_marker_after_commit(db, &publication).map_err(|error| {
            Error::OperationCommittedRepairRequired {
                operation: publication.operation_id.0.clone(),
                repair: "projection marker/runtime".into(),
                reason: error.to_string(),
            }
        })?;
        return Ok(publication);
    }
    if !matches!(
        producer,
        IntentProducer::StructuredPatchProjection
            | IntentProducer::RestoreProjection
            | IntentProducer::CowPublication
            | IntentProducer::ObservedCheckpoint
    ) {
        return Err(Error::InvalidInput(
            "projection-only producer cannot advance a ref".into(),
        ));
    }
    db.changed_path_ledger().recover_scope(expected)?;
    let (operation, operation_id) = db.store_operation_object_unindexed(operation)?;
    let target = IntentTarget {
        change_id: operation.change_id.clone(),
        root_id: operation.after_root.clone(),
        operation_id: Some(operation_id.clone()),
    };
    let intent = super::prepare_intent(
        &db.changed_path_ledger(),
        expected,
        producer,
        &target,
        evidence,
    )?;
    let proof = apply_sync_verify_and_fence(db, &intent)?;
    let _observer_exclusion = crate::db::begin_authorized_observer_write_exclusion(&db.db_dir);
    let _write_lock = acquire_projection_publication_lock(db)?;
    super::mark_filesystem_applied(&db.changed_path_ledger(), expected, &intent, &proof)?;
    let publication = publish_ref_advancing_projection(
        db,
        expected,
        expected_ref,
        lane_id,
        &intent,
        &operation,
        &operation_id,
    )?;
    db.repair_ref_mirror(
        expected_ref,
        &operation.change_id,
        &operation.after_root,
        &operation_id,
    )
    .map_err(|error| Error::OperationCommittedRepairRequired {
        operation: publication.operation_id.0.clone(),
        repair: "ref mirror".into(),
        reason: error.to_string(),
    })?;
    repair_marker_after_commit(db, &publication).map_err(|error| {
        Error::OperationCommittedRepairRequired {
            operation: publication.operation_id.0.clone(),
            repair: "projection marker/runtime".into(),
            reason: error.to_string(),
        }
    })?;
    Ok(publication)
}

/// The non-authoritative fallback used while ledger command authority remains
/// disabled. It still removes the historic ref-before-lane-row visibility
/// window. Task 15 selects the intent-qualified publisher above once native
/// lane observers are platform-qualified.
pub(crate) fn commit_lane_operation_atomic(
    db: &crate::Trail,
    expected_ref: &RefRecord,
    lane_id: &str,
    operation: &Operation,
    workspace_checkpoint: Option<&WorkspaceViewCheckpointPublication>,
) -> Result<ObjectId> {
    if operation.before_root.as_ref() != Some(&expected_ref.root_id)
        || operation.parents.first() != Some(&expected_ref.change_id)
        || operation.branch != expected_ref.name
    {
        return Err(Error::StaleBranch(expected_ref.name.clone()));
    }
    let (operation, operation_id) = db.store_operation_object_unindexed(operation)?;
    let tx = Transaction::new_unchecked(&db.conn, TransactionBehavior::Immediate)?;
    db.index_operation_in_transaction(&operation, &operation_id)?;
    db.advance_ref_cas_in_transaction(
        expected_ref,
        &operation.change_id,
        &operation.after_root,
        &operation_id,
    )?;
    let changed = tx.execute(
        "UPDATE lane_branches SET head_change=?1,head_root=?2,updated_at=?3
         WHERE lane_id=?4 AND ref_name=?5 AND head_change=?6 AND head_root=?7",
        params![
            operation.change_id.0,
            operation.after_root.0,
            now_ts(),
            lane_id,
            expected_ref.name,
            expected_ref.change_id.0,
            expected_ref.root_id.0,
        ],
    )?;
    if changed != 1 {
        return Err(Error::StaleBranch(expected_ref.name.clone()));
    }
    if let Some(checkpoint) = workspace_checkpoint {
        let view_changed = tx.execute(
            "UPDATE workspace_views
             SET checkpoint_seq=?1,checkpoint_root=?2,generation=?3,updated_at=?4
             WHERE view_id=?5 AND lane_id=?6 AND generation=?7",
            params![
                sql_u64(checkpoint.journal_sequence, "workspace journal sequence")?,
                operation.after_root.0,
                sql_u64(checkpoint.next_generation, "workspace view generation")?,
                now_ts(),
                checkpoint.view_id,
                lane_id,
                sql_u64(checkpoint.expected_generation, "workspace view generation")?,
            ],
        )?;
        if view_changed != 1 {
            return Err(Error::StaleBranch(expected_ref.name.clone()));
        }
    }
    tx.commit()?;
    db.repair_ref_mirror(
        expected_ref,
        &operation.change_id,
        &operation.after_root,
        &operation_id,
    )
    .map_err(|error| Error::OperationCommittedRepairRequired {
        operation: operation_id.0.clone(),
        repair: "lane operation ref mirror".into(),
        reason: error.to_string(),
    })?;
    Ok(operation_id)
}

/// Publish an unchanged-ref projection after the filesystem proof has moved
/// the intent to `filesystem_applied`. This is used by checkout alignment,
/// materialization, hydration, and sync. The target must be the exact current
/// authoritative ref/root; projection-only work never fabricates history.
pub(crate) fn publish_alignment_in_transaction(
    db: &crate::Trail,
    expected: &ExpectedScope,
    intent_id: &IntentId,
) -> Result<()> {
    let conn = &db.conn;
    let tx = Transaction::new_unchecked(conn, TransactionBehavior::Immediate)?;
    exact_scope_guard(&tx, expected, true)?;
    let intent = load_intent(&tx, intent_id)?.ok_or_else(|| {
        Error::InvalidInput(format!("unknown changed-path intent `{}`", intent_id.0))
    })?;
    if intent.scope_id != expected.scope_id.to_text()
        || intent.state != IntentState::FilesystemApplied
        || intent.target.operation_id.is_some()
        || intent.target.change_id != intent.expected_change_id
        || intent.target.root_id != intent.expected_root_id
        || intent.target.root_id != expected.baseline_root
    {
        return Err(Error::Conflict(format!(
            "intent `{}` is not an unchanged-ref filesystem alignment",
            intent_id.0
        )));
    }
    let current_ref: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM refs WHERE name=?1 AND generation=?2 AND change_id=?3 AND root_id=?4)",
        params![
            expected.ref_name,
            sql_u64(expected.ref_generation, "ref generation")?,
            intent.expected_change_id.0,
            intent.expected_root_id.0,
        ],
        |row| row.get(0),
    )?;
    if !current_ref {
        return Err(Error::StaleBranch(expected.ref_name.clone()));
    }
    let proof = intent.verified_cut.as_ref().ok_or_else(|| {
        Error::Conflict(format!(
            "intent `{}` has no qualified final cut",
            intent_id.0
        ))
    })?;
    super::intent::validate_qualified_filesystem_proof(
        &tx,
        ledger_database_path(&tx)?,
        expected,
        &intent,
        proof,
    )?;
    let cut_matches: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM changed_path_scopes WHERE scope_id=?1 AND epoch=?2
         AND durable_offset=?3 AND folded_offset=?3)",
        params![
            expected.scope_id.to_text(),
            sql_u64(expected.epoch, "scope epoch")?,
            sql_u64(proof.publication_cut.durable_offset, "provider cut")?,
        ],
        |row| row.get(0),
    )?;
    if !cut_matches {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "alignment lost the exact qualified provider cut".into(),
            command: "trail status".into(),
        });
    }
    acknowledge_controlled_observer_evidence_in_transaction(db, &tx, expected, &intent, proof)?;
    stage_intent_evidence(&tx, &intent)?;
    acknowledge_intent_owned_evidence_in_transaction(&tx, expected, &intent, proof)?;
    terminal_intent(&tx, intent_id)?;
    tx.commit()?;
    super::intent::durable_intent_barrier(conn)
}

/// Atomically publish a ref-advancing projection. The immutable operation
/// object must already exist but must not yet be indexed. On failure it stays
/// unreachable and is reclaimed by ordinary object GC.
pub(crate) fn publish_ref_advancing_projection(
    db: &crate::Trail,
    expected: &ExpectedScope,
    expected_ref: &RefRecord,
    lane_id: &str,
    intent_id: &IntentId,
    operation: &Operation,
    operation_id: &ObjectId,
) -> Result<ProjectionPublication> {
    if operation.before_root.as_ref() != Some(&expected.baseline_root)
        || operation.parents.first() != Some(&expected_ref.change_id)
        || expected_ref.name != expected.ref_name
        || u64::try_from(expected_ref.generation).ok() != Some(expected.ref_generation)
    {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "stale_baseline".into(),
            reason: "ref-advancing projection does not match its prepared scope".into(),
            command: "trail status".into(),
        });
    }
    let tx = Transaction::new_unchecked(&db.conn, TransactionBehavior::Immediate)?;
    exact_scope_guard(&tx, expected, true)?;
    let intent = load_intent(&tx, intent_id)?.ok_or_else(|| {
        Error::InvalidInput(format!("unknown changed-path intent `{}`", intent_id.0))
    })?;
    if intent.scope_id != expected.scope_id.to_text()
        || intent.state != IntentState::FilesystemApplied
        || intent.target.change_id != operation.change_id
        || intent.target.root_id != operation.after_root
        || intent.target.operation_id.as_ref() != Some(operation_id)
    {
        return Err(Error::Conflict(format!(
            "intent `{}` target is not the immutable projection operation",
            intent_id.0
        )));
    }
    let proof = intent.verified_cut.as_ref().ok_or_else(|| {
        Error::Conflict(format!(
            "intent `{}` has no qualified final cut",
            intent_id.0
        ))
    })?;
    super::intent::validate_qualified_filesystem_proof(
        &tx,
        ledger_database_path(&tx)?,
        expected,
        &intent,
        proof,
    )?;

    db.index_operation_in_transaction(operation, operation_id)?;
    db.advance_ref_cas_in_transaction(
        expected_ref,
        &operation.change_id,
        &operation.after_root,
        operation_id,
    )?;
    let lane_changed = tx.execute(
        "UPDATE lane_branches SET head_change=?1,head_root=?2,updated_at=?3
         WHERE lane_id=?4 AND ref_name=?5 AND head_change=?6 AND head_root=?7",
        params![
            operation.change_id.0,
            operation.after_root.0,
            now_ts(),
            lane_id,
            expected_ref.name,
            expected_ref.change_id.0,
            expected_ref.root_id.0,
        ],
    )?;
    if lane_changed != 1 {
        return Err(Error::StaleBranch(expected_ref.name.clone()));
    }
    if !authoritative_ref_matches_target(&tx, &intent)? {
        return Err(Error::StaleBranch(expected_ref.name.clone()));
    }
    acknowledge_controlled_observer_evidence_in_transaction(db, &tx, expected, &intent, proof)?;
    stage_intent_evidence(&tx, &intent)?;
    acknowledge_intent_owned_evidence_in_transaction(&tx, expected, &intent, proof)?;
    let next_generation = expected_ref
        .generation
        .checked_add(1)
        .ok_or_else(|| Error::InvalidInput("ref generation overflow".into()))?;
    let scope_changed = tx.execute(
        "UPDATE changed_path_scopes SET ref_generation=?1,change_id=?2,baseline_root_id=?3,
             updated_at=?4
         WHERE scope_id=?5 AND epoch=?6 AND ref_name=?7 AND ref_generation=?8
           AND baseline_root_id=?9 AND policy_fingerprint=?10
           AND policy_dependency_generation=?11 AND filesystem_identity=?12
           AND provider_identity=?13 AND trust_state='trusted'
           AND durable_offset=?14 AND folded_offset=?14",
        params![
            next_generation,
            operation.change_id.0,
            operation.after_root.0,
            now_ts(),
            expected.scope_id.to_text(),
            sql_u64(expected.epoch, "scope epoch")?,
            expected.ref_name,
            expected_ref.generation,
            expected.baseline_root.0,
            hex::encode(expected.policy_fingerprint),
            sql_u64(expected.policy_generation, "policy generation")?,
            hex::encode(&expected.filesystem_identity),
            hex::encode(&expected.provider_identity),
            sql_u64(proof.publication_cut.durable_offset, "provider cut")?,
        ],
    )?;
    if scope_changed != 1 {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "stale_baseline".into(),
            reason: "projection publication lost the exact scope CAS".into(),
            command: "trail status".into(),
        });
    }
    terminal_intent(&tx, intent_id)?;
    tx.commit()?;
    super::intent::durable_intent_barrier(&db.conn)?;
    Ok(ProjectionPublication {
        operation_id: operation_id.clone(),
        baseline: BaselineIdentity {
            ref_name: expected_ref.name.clone(),
            ref_generation: u64::try_from(next_generation)
                .map_err(|_| Error::InvalidInput("ref generation overflow".into()))?,
            change_id: operation.change_id.clone(),
            root_id: operation.after_root.clone(),
        },
    })
}

/// Remove immutable observer-only evidence for intended paths only when the
/// entire row falls inside the authenticated controlled interval. Pre-existing,
/// mixed-source, and post-c1 rows remain pending.
fn acknowledge_controlled_observer_evidence_in_transaction(
    db: &crate::Trail,
    tx: &rusqlite::Connection,
    expected: &ExpectedScope,
    intent: &super::intent::PersistedIntent,
    proof: &QualifiedFilesystemProof,
) -> Result<()> {
    let mut tokens = Vec::new();
    let mut exact = tx.prepare(
        "SELECT entry.normalized_path,entry.event_flags,entry.source_mask,
                entry.first_sequence,entry.last_sequence,entry.provider_id,
                entry.provider_sequence,entry.intent_id
         FROM changed_path_entries entry
         JOIN changed_path_intent_paths path
           ON path.intent_id=?1 AND path.normalized_path=entry.normalized_path
         WHERE entry.scope_id=?2 AND entry.source_mask=?3
           AND entry.first_sequence>=?4 AND entry.last_sequence<=?5
           AND entry.provider_sequence IS NOT NULL
           AND (entry.event_flags & path.event_flags)!=0
         ORDER BY entry.normalized_path COLLATE BINARY",
    )?;
    let rows = exact.query_map(
        params![
            intent.id.0,
            expected.scope_id.to_text(),
            EvidenceSource::Observer.mask(),
            sql_u64(proof.start_sequence, "controlled start sequence")?,
            sql_u64(proof.end_cut.sequence, "controlled end sequence")?,
        ],
        |row| acknowledgement_token(row, EvidenceRowKind::Exact),
    )?;
    tokens.extend(rows.collect::<std::result::Result<Vec<_>, _>>()?);
    drop(exact);
    let mut prefixes = tx.prepare(
        "SELECT entry.normalized_prefix,entry.event_flags,entry.source_mask,
                entry.first_sequence,entry.last_sequence,entry.provider_id,
                entry.provider_sequence,entry.intent_id
         FROM changed_path_prefixes entry
         JOIN changed_path_intent_prefixes prefix
           ON prefix.intent_id=?1 AND prefix.normalized_prefix=entry.normalized_prefix
         WHERE entry.scope_id=?2 AND entry.source_mask=?3
           AND entry.first_sequence>=?4 AND entry.last_sequence<=?5
           AND entry.provider_sequence IS NOT NULL
           AND (entry.event_flags & prefix.event_flags)!=0
           AND prefix.completeness_reason='provider_complete'
         ORDER BY entry.normalized_prefix COLLATE BINARY",
    )?;
    let rows = prefixes.query_map(
        params![
            intent.id.0,
            expected.scope_id.to_text(),
            EvidenceSource::Observer.mask(),
            sql_u64(proof.start_sequence, "controlled start sequence")?,
            sql_u64(proof.end_cut.sequence, "controlled end sequence")?,
        ],
        |row| acknowledgement_token(row, EvidenceRowKind::CompletePrefix),
    )?;
    tokens.extend(rows.collect::<std::result::Result<Vec<_>, _>>()?);
    drop(prefixes);
    let intended = tx
        .prepare(
            "SELECT normalized_path FROM changed_path_intent_paths
             WHERE intent_id=?1 ORDER BY normalized_path COLLATE BINARY",
        )?
        .query_map([&intent.id.0], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<BTreeSet<_>, _>>()?;
    let mut internal = tx.prepare(
        "SELECT normalized_path,event_flags,source_mask,first_sequence,last_sequence,
                provider_id,provider_sequence,intent_id
         FROM changed_path_entries
         WHERE scope_id=?1 AND source_mask=?2 AND intent_id IS NULL
           AND first_sequence>=?3 AND last_sequence<=?4
           AND provider_sequence IS NOT NULL
         ORDER BY normalized_path COLLATE BINARY",
    )?;
    let rows = internal.query_map(
        params![
            expected.scope_id.to_text(),
            EvidenceSource::Observer.mask(),
            sql_u64(proof.start_sequence, "controlled start sequence")?,
            sql_u64(proof.end_cut.sequence, "controlled end sequence")?,
        ],
        |row| acknowledgement_token(row, EvidenceRowKind::Exact),
    )?;
    let mut internal_tokens = Vec::new();
    for token in rows {
        let token = token?;
        if trail_case_probe_token(&token)
            || trail_atomic_temp_target(&token)
                .as_ref()
                .is_some_and(|target| intended.contains(target))
        {
            internal_tokens.push(token);
        }
    }
    drop(internal);
    let baseline_internal = db.load_root_files_for_paths(
        &expected.baseline_root,
        &internal_tokens
            .iter()
            .map(|token| token.path.as_str().to_string())
            .collect::<Vec<_>>(),
    )?;
    tokens.extend(
        internal_tokens
            .into_iter()
            .filter(|token| !baseline_internal.contains_key(token.path.as_str())),
    );
    tokens.sort_by(|left, right| left.path.as_str().cmp(right.path.as_str()));
    tokens.dedup();
    super::ChangedPathLedger::acknowledge_immutable_tokens_in_transaction(
        tx,
        expected,
        &proof.publication_cut,
        proof.end_cut.sequence,
        &tokens,
        true,
    )?;
    Ok(())
}

/// Remove the immutable intent-only rows after controlled observer evidence
/// has been acknowledged. Any merge makes the row mixed-source and preserves
/// it for the next snapshot.
pub(super) fn acknowledge_intent_owned_evidence_in_transaction(
    tx: &rusqlite::Connection,
    expected: &ExpectedScope,
    intent: &super::intent::PersistedIntent,
    proof: &QualifiedFilesystemProof,
) -> Result<()> {
    let mut tokens = Vec::new();
    let mut exact = tx.prepare(
        "SELECT entry.normalized_path,entry.event_flags,entry.source_mask,
                entry.first_sequence,entry.last_sequence,entry.provider_id,
                entry.provider_sequence,entry.intent_id
         FROM changed_path_entries entry
         JOIN changed_path_intent_paths path
           ON path.intent_id=?1 AND path.normalized_path=entry.normalized_path
         WHERE entry.scope_id=?2 AND entry.source_mask=?3 AND entry.intent_id=?1
           AND entry.first_sequence=?4 AND entry.last_sequence=?4
           AND entry.provider_sequence IS NULL
           AND (entry.event_flags & path.event_flags)=path.event_flags
         ORDER BY entry.normalized_path COLLATE BINARY",
    )?;
    let rows = exact.query_map(
        params![
            intent.id.0,
            expected.scope_id.to_text(),
            EvidenceSource::Intent.mask(),
            sql_u64(proof.end_cut.sequence, "intent end sequence")?,
        ],
        |row| acknowledgement_token(row, EvidenceRowKind::Exact),
    )?;
    tokens.extend(rows.collect::<std::result::Result<Vec<_>, _>>()?);
    drop(exact);
    let mut prefixes = tx.prepare(
        "SELECT entry.normalized_prefix,entry.event_flags,entry.source_mask,
                entry.first_sequence,entry.last_sequence,entry.provider_id,
                entry.provider_sequence,entry.intent_id
         FROM changed_path_prefixes entry
         JOIN changed_path_intent_prefixes prefix
           ON prefix.intent_id=?1 AND prefix.normalized_prefix=entry.normalized_prefix
         WHERE entry.scope_id=?2 AND entry.source_mask=?3 AND entry.intent_id=?1
           AND entry.first_sequence=?4 AND entry.last_sequence=?4
           AND entry.provider_sequence IS NULL
           AND (entry.event_flags & prefix.event_flags)=prefix.event_flags
           AND prefix.completeness_reason='provider_complete'
         ORDER BY entry.normalized_prefix COLLATE BINARY",
    )?;
    let rows = prefixes.query_map(
        params![
            intent.id.0,
            expected.scope_id.to_text(),
            EvidenceSource::Intent.mask(),
            sql_u64(proof.end_cut.sequence, "intent end sequence")?,
        ],
        |row| acknowledgement_token(row, EvidenceRowKind::CompletePrefix),
    )?;
    tokens.extend(rows.collect::<std::result::Result<Vec<_>, _>>()?);
    drop(prefixes);
    super::ChangedPathLedger::acknowledge_immutable_tokens_in_transaction(
        tx,
        expected,
        &proof.publication_cut,
        proof.end_cut.sequence,
        &tokens,
        true,
    )?;
    Ok(())
}

fn acknowledgement_token(
    row: &rusqlite::Row<'_>,
    kind: EvidenceRowKind,
) -> rusqlite::Result<EvidenceAcknowledgementToken> {
    let first = row.get::<_, i64>(3)?;
    let last = row.get::<_, i64>(4)?;
    let provider_sequence = row.get::<_, Option<i64>>(6)?;
    Ok(EvidenceAcknowledgementToken {
        kind,
        path: LedgerPath(row.get(0)?),
        flags: EvidenceFlags(row.get(1)?),
        source_mask: row.get(2)?,
        first_sequence: u64::try_from(first).unwrap_or(u64::MAX),
        last_sequence: u64::try_from(last).unwrap_or(u64::MAX),
        provider_id: row.get(5)?,
        provider_sequence: provider_sequence.and_then(|value| u64::try_from(value).ok()),
        intent_id: row.get(7)?,
    })
}

fn terminal_intent(tx: &rusqlite::Connection, intent_id: &IntentId) -> Result<()> {
    let changed = tx.execute(
        "UPDATE changed_path_intents SET lifecycle_state='acknowledged',updated_at=?1
         WHERE intent_id=?2 AND lifecycle_state='filesystem_applied'",
        params![now_ts(), intent_id.0],
    )?;
    if changed != 1 {
        return Err(Error::Conflict(
            "projection intent terminal transition raced".into(),
        ));
    }
    Ok(())
}

fn sql_u64(value: u64, label: &str) -> Result<i64> {
    value
        .try_into()
        .map_err(|_| Error::InvalidInput(format!("{label} exceeds SQLite INTEGER range")))
}

fn ledger_database_path(conn: &rusqlite::Connection) -> Result<&std::path::Path> {
    let path = conn.path().ok_or_else(|| {
        Error::InvalidInput("changed-path publication database path is unavailable".into())
    })?;
    Ok(std::path::Path::new(path))
}
