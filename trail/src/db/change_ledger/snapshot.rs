//! Fenced changed-path command snapshots.
//!
//! Production command authority remains gated until Task 15.  This module
//! owns the state machine and data types so status, diff, and record cannot
//! accidentally assemble a partially-fenced result.

use super::{
    BaselineIdentity, CandidateSnapshot, EvidenceAcknowledgementToken, EvidenceCut, ExpectedScope,
};
use crate::db::storage::file_kind_from_index;
use crate::db::{DiskFile, DiskManifest, OperationMetricsDelta};
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
    pub(crate) disk_files: Option<Vec<DiskFile>>,
    pub(crate) summaries: Vec<FileDiffSummary>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CandidateMaterialization {
    ManifestOnly,
    RecordBytes,
}

impl CandidateMaterialization {
    fn retains_bytes(self) -> bool {
        matches!(self, Self::RecordBytes)
    }
}

impl crate::Trail {
    pub(crate) fn compare_authoritative_candidates(
        &self,
        policy: &super::CompiledPolicy,
        snapshot: &CandidateSnapshot,
        root_id: &ObjectId,
        materialization: CandidateMaterialization,
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
                disk_files: materialization.retains_bytes().then(Vec::new),
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
        let mut disk_files = materialization.retains_bytes().then(Vec::new);
        for path in exact {
            if matcher.is_ignored(&path, false)? {
                continue;
            }
            if let Some(file) = self.read_pinned_candidate_path(
                &pinned,
                policy,
                &path,
                materialization.retains_bytes(),
            )? {
                let bytes = file.bytes;
                if let Some(disk_files) = disk_files.as_mut() {
                    disk_files.push(DiskFile {
                        path: path.clone(),
                        bytes: bytes.ok_or_else(|| {
                            crate::Error::Corrupt(
                                "record candidate read omitted captured contents".into(),
                            )
                        })?,
                        executable: file.executable,
                    });
                } else if bytes.is_some() {
                    return Err(crate::Error::Corrupt(
                        "manifest-only candidate comparison retained file contents".into(),
                    ));
                }
                disk_manifest.insert(
                    path.clone(),
                    DiskManifest {
                        kind: file_kind_from_index(&file.file_kind)?,
                        executable: file.executable,
                        content_hash: file.content_hash,
                    },
                );
            }
        }
        self.visit_pinned_worktree_prefix_files(
            &pinned,
            &matcher,
            &prefixes,
            materialization.retains_bytes(),
            |file| {
                let bytes = file.bytes;
                if let Some(disk_files) = disk_files.as_mut() {
                    disk_files.push(DiskFile {
                        path: file.path.clone(),
                        bytes: bytes.ok_or_else(|| {
                            crate::Error::Corrupt(
                                "record prefix read omitted captured contents".into(),
                            )
                        })?,
                        executable: file.executable,
                    });
                } else if bytes.is_some() {
                    return Err(crate::Error::Corrupt(
                        "manifest-only prefix comparison retained file contents".into(),
                    ));
                }
                disk_manifest.insert(
                    file.path.clone(),
                    DiskManifest {
                        kind: file_kind_from_index(&file.file_kind)?,
                        executable: file.executable,
                        content_hash: file.content_hash,
                    },
                );
                Ok(())
            },
        )?;
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
            disk_files,
            summaries,
        })
    }
}

#[cfg(debug_assertions)]
pub(crate) fn run_command_flow() -> std::result::Result<(), String> {
    run_command_flow_inner(false)
}

#[cfg(debug_assertions)]
pub(crate) fn run_command_long_lock_flow() -> std::result::Result<(), String> {
    run_command_flow_inner(true)
}

#[cfg(debug_assertions)]
fn run_command_flow_inner(long_record_lock: bool) -> std::result::Result<(), String> {
    use crate::model::Actor;
    use crate::{InitImportMode, Trail};
    use sha2::{Digest, Sha256};
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
    fs::create_dir(temp.path().join("tree")).map_err(|error| error.to_string())?;
    fs::write(temp.path().join("tree/nested.txt"), vec![b'x'; 256 * 1024])
        .map_err(|error| error.to_string())?;
    fs::write(temp.path().join("tracked.txt"), b"baseline\n").map_err(|error| error.to_string())?;
    let mut db = Trail::open(temp.path()).map_err(|error| error.to_string())?;
    db.record(None, Some("baseline".into()), Actor::human(), false)
        .map_err(|error| error.to_string())?;
    super::prepare_workspace_daemon(&mut db, false).map_err(|error| error.to_string())?;
    set_command_authority_override(true);
    let _reset = OverrideReset;

    let (scope_id, provider_id): (String, String) = db
        .conn
        .query_row(
            "SELECT scope_id,provider_id FROM changed_path_scopes
             WHERE scope_kind='workspace'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    db.conn
        .execute(
            "INSERT INTO changed_path_prefixes(
                 scope_id,normalized_prefix,completeness_reason,event_flags,source_mask,
                 first_sequence,last_sequence,provider_id,provider_sequence,created_at,updated_at
             ) VALUES(?1,'tree','provider_complete',?2,?3,1,1,?4,1,
                      strftime('%s','now'),strftime('%s','now'))",
            params![
                scope_id,
                super::EvidenceFlags::PROVIDER_COMPLETE_PREFIX.0,
                super::EvidenceSource::Observer.mask(),
                provider_id,
            ],
        )
        .map_err(|error| error.to_string())?;

    let (metadata_only, _) = db
        .with_workspace_authoritative_snapshot(|db, policy, candidates| {
            db.compare_authoritative_candidates(
                policy,
                candidates,
                &candidates.expected.baseline_root,
                CandidateMaterialization::ManifestOnly,
            )
        })
        .map_err(|error| format!("metadata-only complete-prefix comparison: {error}"))?;
    if metadata_only.disk_files.is_some()
        || !metadata_only.disk_manifest.contains_key("tree/nested.txt")
    {
        return Err("metadata-only complete-prefix comparison retained content bytes".into());
    }
    let (record_bytes, _) = db
        .with_workspace_authoritative_snapshot(|db, policy, candidates| {
            db.compare_authoritative_candidates(
                policy,
                candidates,
                &candidates.expected.baseline_root,
                CandidateMaterialization::RecordBytes,
            )
        })
        .map_err(|error| format!("record-byte complete-prefix comparison: {error}"))?;
    let retained = record_bytes.disk_files.as_ref().ok_or_else(|| {
        "record-byte complete-prefix comparison omitted its explicit materialization".to_string()
    })?;
    if retained.len() != 1 || retained[0].path != "tree/nested.txt" {
        return Err("record-byte complete-prefix comparison retained the wrong path set".into());
    }

    let consume_attempts = std::cell::Cell::new(0_u8);
    db.with_workspace_authoritative_snapshot(|_, _, candidates| {
        let attempt = consume_attempts.get().saturating_add(1);
        consume_attempts.set(attempt);
        if attempt == 1 {
            return Err(crate::Error::ChangeLedgerReconcileRequired {
                scope: candidates.expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: "command_test_consumer_retry".into(),
                command: "trail status".into(),
            });
        }
        Ok(())
    })
    .map_err(|error| format!("consumer reconciliation retry: {error}"))?;
    if consume_attempts.get() != 2 {
        return Err("consumer reconciliation did not retry the entire fenced flow once".into());
    }

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
    let changed_after_comparison = temp.path().join("tracked.txt");
    crate::db::install_observed_record_after_compare_hook(move || {
        fs::write(changed_after_comparison, b"changed-after-comparison\n")?;
        Ok(())
    });
    if long_record_lock {
        let changed_with_lock = temp.path().join("tracked.txt");
        crate::db::install_observed_record_with_lock_hook(move || {
            fs::write(changed_with_lock, b"changed-while-record-lock-held\n")?;
            std::thread::sleep(std::time::Duration::from_millis(5_500));
            Ok(())
        });
    }
    let recorded = db
        .record(None, Some("observed".into()), Actor::human(), false)
        .map_err(|error| format!("observed record: {error}"))?;
    if recorded.operation.is_none() || recorded.changed_paths.len() != 1 {
        return Err("observed record did not publish the candidate change".into());
    }
    let recorded_files = db
        .load_root_files_for_paths(&recorded.root_id, &["tracked.txt".to_string()])
        .map_err(|error| format!("load observed record root: {error}"))?;
    let recorded_hash = recorded_files
        .get("tracked.txt")
        .map(|entry| entry.content_hash.as_str());
    let compared_hash = hex::encode(Sha256::digest(b"changed\n"));
    if recorded_hash != Some(compared_hash.as_str()) {
        return Err("observed record did not use the authorized pinned candidate bytes".into());
    }
    let after_race = db
        .status_from_changed_path_ledger()
        .map_err(|error| format!("post-record authoritative status: {error}"))?;
    if after_race.changed_paths.len() != 1 || after_race.changed_paths[0].path != "tracked.txt" {
        return Err("post-comparison mutation was not retained as c2 evidence".into());
    }
    let recorded_after_race = db
        .record(
            None,
            Some("observed-after-race".into()),
            Actor::human(),
            false,
        )
        .map_err(|error| format!("observed record after comparison race: {error}"))?;
    if recorded_after_race.operation.is_none() || recorded_after_race.changed_paths.len() != 1 {
        return Err("second observed record did not publish retained c2 evidence".into());
    }
    let after = db
        .status_from_changed_path_ledger()
        .map_err(|error| format!("final post-record authoritative status: {error}"))?;
    if !after.changed_paths.is_empty() {
        return Err("atomic observed record did not acknowledge unchanged evidence".into());
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
