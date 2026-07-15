//! Fenced changed-path command snapshots.
//!
//! Production command authority remains gated until Task 15.  This module
//! owns the state machine and data types so status, diff, and record cannot
//! accidentally assemble a partially-fenced result.

use super::{
    BaselineIdentity, CandidateSnapshot, EvidenceAcknowledgementToken, EvidenceCut, ExpectedScope,
};
use crate::db::storage::file_kind_from_index;
use crate::db::{DiskManifest, OperationMetricsDelta};
use crate::model::{FileDiffSummary, FileEntry};
use crate::model::{Operation, RefRecord};
use crate::{ObjectId, Result};
use rusqlite::params;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};

static COMMAND_AUTHORITY_OVERRIDE: AtomicBool = AtomicBool::new(false);

/// Task 15 replaces the hard-off production branch after platform
/// qualification.  Debug tests may exercise the fully wired command path
/// without silently activating it for users.
pub(crate) fn command_authority_enabled() -> bool {
    cfg!(debug_assertions) && COMMAND_AUTHORITY_OVERRIDE.load(Ordering::Acquire)
}

#[cfg(debug_assertions)]
pub(crate) fn set_command_authority_override(enabled: bool) {
    COMMAND_AUTHORITY_OVERRIDE.store(enabled, Ordering::Release);
}

#[derive(Clone, Debug)]
pub(crate) struct ObservedRecordCut {
    pub(crate) expected: ExpectedScope,
    pub(crate) c1: EvidenceCut,
    pub(crate) c2: EvidenceCut,
    pub(crate) acknowledgement_tokens: Vec<EvidenceAcknowledgementToken>,
}

#[derive(Clone, Debug)]
pub(crate) struct FencedCandidateSnapshot {
    pub(crate) candidates: CandidateSnapshot,
    pub(crate) c2: EvidenceCut,
}

#[derive(Clone, Debug)]
pub(crate) struct CandidateComparison {
    pub(crate) selections: Vec<String>,
    pub(crate) baseline_files: BTreeMap<String, FileEntry>,
    pub(crate) disk_manifest: BTreeMap<String, DiskManifest>,
    pub(crate) summaries: Vec<FileDiffSummary>,
}

impl crate::Trail {
    pub(crate) fn compare_authoritative_candidates(
        &self,
        policy: &super::CompiledPolicy,
        snapshot: &CandidateSnapshot,
        root_id: &ObjectId,
    ) -> Result<CandidateComparison> {
        if snapshot.expected.baseline_root != *root_id
            || snapshot.expected.policy_fingerprint != policy.fingerprint()
        {
            return Err(crate::Error::ChangeLedgerReconcileRequired {
                scope: snapshot.expected.scope_id.to_text(),
                state: "stale_baseline".into(),
                reason: "candidate comparison baseline or compiled policy changed".into(),
                command: "trail status".into(),
            });
        }
        let mut exact = snapshot
            .exact_paths
            .iter()
            .map(|path| path.as_str().to_string())
            .collect::<BTreeSet<_>>();
        let prefixes = snapshot
            .prefixes
            .iter()
            .filter(|prefix| prefix.complete)
            .map(|prefix| prefix.path.as_str().to_string())
            .collect::<Vec<_>>();
        exact.retain(|path| {
            !prefixes.iter().any(|prefix| {
                path == prefix
                    || path
                        .strip_prefix(prefix)
                        .is_some_and(|rest| rest.starts_with('/'))
            })
        });
        let mut selections = exact.iter().cloned().collect::<Vec<_>>();
        selections.extend(prefixes.iter().cloned());
        selections.sort();
        selections.dedup();
        self.note_operation_metrics(OperationMetricsDelta {
            authoritative_candidate_count: selections.len().try_into().unwrap_or(u64::MAX),
            ..OperationMetricsDelta::default()
        });
        let baseline_files = self.load_root_files_for_selections(root_id, &selections)?;
        if selections.is_empty() {
            return Ok(CandidateComparison {
                selections,
                baseline_files,
                disk_manifest: BTreeMap::new(),
                summaries: Vec::new(),
            });
        }
        let matcher = policy.recording_matcher()?;
        let pinned = self.open_pinned_worktree_root(policy)?;
        if self.pinned_worktree_root_identity(&pinned) != snapshot.expected.filesystem_identity {
            return Err(crate::Error::ChangeLedgerReconcileRequired {
                scope: snapshot.expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: "pinned workspace root identity changed before candidate comparison".into(),
                command: "trail status".into(),
            });
        }
        let mut disk_manifest = BTreeMap::new();
        for path in exact {
            if matcher.is_ignored(&path, false)? {
                continue;
            }
            if let Some(file) = self.read_pinned_worktree_path(&pinned, policy, &path)? {
                disk_manifest.insert(
                    path,
                    DiskManifest {
                        kind: file_kind_from_index(&file.file_kind)?,
                        executable: file.executable,
                        content_hash: file.content_hash,
                    },
                );
            }
        }
        self.visit_pinned_worktree_prefix_files(&pinned, &matcher, &prefixes, |file| {
            disk_manifest.insert(
                file.path,
                DiskManifest {
                    kind: file_kind_from_index(&file.file_kind)?,
                    executable: file.executable,
                    content_hash: file.content_hash,
                },
            );
            Ok(())
        })?;
        if !self.verify_pinned_worktree_root(&pinned)? {
            return Err(crate::Error::ChangeLedgerReconcileRequired {
                scope: snapshot.expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: "workspace root identity changed during candidate comparison".into(),
                command: "trail status".into(),
            });
        }
        let summaries = self.diff_file_maps_to_manifest_for_paths(
            &baseline_files,
            &disk_manifest,
            &selections,
        )?;
        Ok(CandidateComparison {
            selections,
            baseline_files,
            disk_manifest,
            summaries,
        })
    }
}

#[cfg(debug_assertions)]
pub(crate) fn run_command_flow() -> std::result::Result<(), String> {
    use crate::model::Actor;
    use crate::{InitImportMode, Trail};
    use std::fs;
    use std::sync::{Mutex, OnceLock};

    static COMMAND_FLOW: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = COMMAND_FLOW
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    struct OverrideReset;
    impl Drop for OverrideReset {
        fn drop(&mut self) {
            set_command_authority_override(false);
        }
    }

    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    Trail::init(temp.path(), "main", InitImportMode::Empty, false)
        .map_err(|error| error.to_string())?;
    fs::write(temp.path().join("tracked.txt"), b"baseline\n").map_err(|error| error.to_string())?;
    let mut db = Trail::open(temp.path()).map_err(|error| error.to_string())?;
    db.record(None, Some("baseline".into()), Actor::human(), false)
        .map_err(|error| error.to_string())?;
    super::prepare_workspace_daemon(&mut db, false).map_err(|error| error.to_string())?;
    set_command_authority_override(true);
    let _reset = OverrideReset;

    let clean = db
        .status_from_changed_path_ledger()
        .map_err(|error| format!("initial authoritative status: {error}"))?;
    if !clean.changed_paths.is_empty() {
        return Err("empty authoritative candidate set was not clean".into());
    }
    db.conn
        .execute(
            "UPDATE changed_path_scopes SET trust_state='untrusted_gap',
                 trust_reason='command_test_auto_reconcile',
                 continuity_generation=continuity_generation+1",
            [],
        )
        .map_err(|error| error.to_string())?;
    let recovered = db
        .status_from_changed_path_ledger()
        .map_err(|error| format!("automatic reconciliation retry: {error}"))?;
    if !recovered.changed_paths.is_empty() {
        return Err("automatic reconciliation retry did not restore clean authority".into());
    }
    fs::write(temp.path().join("tracked.txt"), b"changed\n").map_err(|error| error.to_string())?;
    let dirty = db
        .status_from_changed_path_ledger()
        .map_err(|error| format!("dirty authoritative status: {error}"))?;
    if dirty.changed_paths.len() != 1 || dirty.changed_paths[0].path != "tracked.txt" {
        return Err(format!(
            "authoritative candidate comparison missed tracked change: {:?}",
            dirty
                .changed_paths
                .iter()
                .map(|item| item.path.as_str())
                .collect::<Vec<_>>()
        ));
    }
    let recorded = db
        .record(None, Some("observed".into()), Actor::human(), false)
        .map_err(|error| format!("observed record: {error}"))?;
    if recorded.operation.is_none() || recorded.changed_paths.len() != 1 {
        return Err("observed record did not publish the candidate change".into());
    }
    let after = db
        .status_from_changed_path_ledger()
        .map_err(|error| format!("post-record authoritative status: {error}"))?;
    if !after.changed_paths.is_empty() {
        return Err("atomic observed record did not acknowledge its unchanged c1 evidence".into());
    }
    Ok(())
}

impl crate::Trail {
    pub(crate) fn with_workspace_authoritative_snapshot<T, F>(
        &mut self,
        consume: F,
    ) -> crate::Result<(T, FencedCandidateSnapshot)>
    where
        F: FnMut(&crate::Trail, &super::CompiledPolicy, &CandidateSnapshot) -> crate::Result<T>,
    {
        let mut runtime = self.changed_path_daemon_runtime.take().ok_or_else(|| {
            crate::Error::DaemonUnavailable("changed-path observer runtime is unavailable".into())
        })?;
        let result = runtime.with_authoritative_snapshot(self, consume);
        self.changed_path_daemon_runtime = Some(runtime);
        result
    }

    /// Publish an observed workspace record as one SQLite transaction.  The
    /// immutable operation object is written first, but remains unreachable if
    /// any index/ref/baseline/ack/lane CAS fails.  The ref mirror and in-memory
    /// daemon baseline are repaired only after commit.
    pub(crate) fn commit_observed_record(
        &mut self,
        operation: &Operation,
        expected_ref: &RefRecord,
        observed: &ObservedRecordCut,
        acknowledge_complete_prefixes: bool,
        lane_id: Option<&str>,
    ) -> Result<ObjectId> {
        if operation.before_root.as_ref() != Some(&observed.expected.baseline_root)
            || operation.parents.as_slice() != [expected_ref.change_id.clone()]
            || expected_ref.name != observed.expected.ref_name
            || u64::try_from(expected_ref.generation).ok() != Some(observed.expected.ref_generation)
            || observed.c1.durable_offset > observed.c2.durable_offset
            || observed.c1.sequence > observed.c2.sequence
        {
            return Err(crate::Error::ChangeLedgerReconcileRequired {
                scope: observed.expected.scope_id.to_text(),
                state: "stale_baseline".into(),
                reason: "observed record does not match its fenced baseline and cuts".into(),
                command: "trail status".into(),
            });
        }
        let (operation, operation_id) = self.store_operation_object_unindexed(operation)?;
        self.conn.execute_batch("BEGIN IMMEDIATE;")?;
        let transaction_result = (|| {
            self.index_operation_in_transaction(&operation, &operation_id)?;
            self.advance_ref_cas_in_transaction(
                expected_ref,
                &operation.change_id,
                &operation.after_root,
                &operation_id,
            )?;
            let touched = super::ChangedPathLedger::acknowledge_immutable_tokens_in_transaction(
                &self.conn,
                &observed.expected,
                &observed.c2,
                observed.c1.sequence,
                &observed.acknowledgement_tokens,
                acknowledge_complete_prefixes,
            )?;
            let changed = self.conn.execute(
                "UPDATE changed_path_scopes SET ref_generation=?1,change_id=?2,
                     baseline_root_id=?3,updated_at=strftime('%s','now')
                 WHERE scope_id=?4 AND epoch=?5 AND ref_name=?6 AND ref_generation=?7
                   AND baseline_root_id=?8 AND policy_fingerprint=?9
                   AND policy_dependency_generation=?10 AND filesystem_identity=?11
                   AND provider_identity=?12 AND trust_state='trusted'
                   AND durable_offset=?13 AND folded_offset=?13",
                params![
                    expected_ref.generation + 1,
                    operation.change_id.0,
                    operation.after_root.0,
                    observed.expected.scope_id.to_text(),
                    i64::try_from(observed.expected.epoch)
                        .map_err(|_| crate::Error::InvalidInput("scope epoch overflow".into()))?,
                    observed.expected.ref_name,
                    expected_ref.generation,
                    observed.expected.baseline_root.0,
                    hex::encode(observed.expected.policy_fingerprint),
                    i64::try_from(observed.expected.policy_generation).map_err(|_| {
                        crate::Error::InvalidInput("policy generation overflow".into())
                    })?,
                    hex::encode(&observed.expected.filesystem_identity),
                    hex::encode(&observed.expected.provider_identity),
                    i64::try_from(observed.c2.durable_offset)
                        .map_err(|_| crate::Error::InvalidInput("observer cut overflow".into()))?,
                ],
            )?;
            if changed != 1 {
                return Err(crate::Error::ChangeLedgerReconcileRequired {
                    scope: observed.expected.scope_id.to_text(),
                    state: "stale_baseline".into(),
                    reason: "observed record lost the latest c2 scope CAS".into(),
                    command: "trail status".into(),
                });
            }
            if let Some(lane_id) = lane_id {
                let changed = self.conn.execute(
                    "UPDATE lane_branches SET head_change=?1,head_root=?2,updated_at=strftime('%s','now')
                     WHERE lane_id=?3 AND ref_name=?4 AND head_change=?5 AND head_root=?6",
                    params![
                        operation.change_id.0,
                        operation.after_root.0,
                        lane_id,
                        expected_ref.name,
                        expected_ref.change_id.0,
                        expected_ref.root_id.0,
                    ],
                )?;
                if changed != 1 {
                    return Err(crate::Error::StaleBranch(expected_ref.name.clone()));
                }
            }
            self.note_operation_metrics(OperationMetricsDelta {
                ledger_row_touch_count: touched,
                ..OperationMetricsDelta::default()
            });
            Ok(())
        })();
        match transaction_result {
            Ok(()) => self.conn.execute_batch("COMMIT;")?,
            Err(error) => {
                let _ = self.conn.execute_batch("ROLLBACK;");
                return Err(error);
            }
        }
        self.repair_ref_mirror(
            expected_ref,
            &operation.change_id,
            &operation.after_root,
            &operation_id,
        )?;
        let target = BaselineIdentity {
            ref_name: expected_ref.name.clone(),
            ref_generation: u64::try_from(expected_ref.generation + 1)
                .map_err(|_| crate::Error::InvalidInput("ref generation overflow".into()))?,
            change_id: operation.change_id.clone(),
            root_id: operation.after_root.clone(),
        };
        if let Some(runtime) = self.changed_path_daemon_runtime.as_mut() {
            runtime.accept_observed_baseline(&observed.expected, &target)?;
        }
        Ok(operation_id)
    }
}
