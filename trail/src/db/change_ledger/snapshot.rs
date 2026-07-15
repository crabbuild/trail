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

fn sparse_selection_intersects(selection: &[String], candidate: &str) -> bool {
    selection.iter().any(|selected| {
        candidate == selected
            || candidate
                .strip_prefix(selected)
                .is_some_and(|rest| rest.starts_with('/'))
            || selected
                .strip_prefix(candidate)
                .is_some_and(|rest| rest.starts_with('/'))
    })
}

impl crate::Trail {
    pub(crate) fn compare_controlled_projection_target(
        &self,
        policy: &super::CompiledPolicy,
        snapshot: &CandidateSnapshot,
        target_root: &ObjectId,
        materialization: CandidateMaterialization,
    ) -> Result<CandidateComparison> {
        if snapshot.expected.policy_fingerprint != policy.fingerprint() {
            return Err(crate::Error::ChangeLedgerReconcileRequired {
                scope: snapshot.expected.scope_id.to_text(),
                state: "stale_baseline".into(),
                reason: "controlled projection policy changed before pinned verification".into(),
                command: "trail status".into(),
            });
        }
        // The immutable target root was prebuilt before the intent. Candidate
        // evidence and filesystem identity remain bound to the original scope;
        // only the comparison baseline is substituted to prove the applied
        // bytes equal that target before c2.
        let mut target_snapshot = snapshot.clone();
        target_snapshot.expected.baseline_root = target_root.clone();
        self.compare_authoritative_candidates(
            policy,
            &target_snapshot,
            target_root,
            materialization,
        )
    }

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
pub(crate) fn run_materialized_lane_snapshot_flow() -> std::result::Result<(), String> {
    use crate::{InitImportMode, Trail};
    use sha2::{Digest, Sha256};
    use std::fs;

    let temp = tempfile::tempdir().map_err(|error| error.to_string())?;
    fs::write(temp.path().join("tracked.txt"), b"baseline\n").map_err(|error| error.to_string())?;
    Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false)
        .map_err(|error| error.to_string())?;
    let mut db = Trail::open(temp.path()).map_err(|error| error.to_string())?;
    let lane = db
        .spawn_lane("ledger-lane", Some("main"), true, None, None)
        .map_err(|error| error.to_string())?;
    let workdir = std::path::PathBuf::from(
        lane.workdir
            .ok_or_else(|| "test lane was not materialized".to_string())?,
    );

    // Make the same path diverge in opposite directions. A comparison that
    // accidentally pins Trail's primary workspace will report the workspace
    // hash instead of the lane hash.
    fs::write(temp.path().join("tracked.txt"), b"workspace-only\n")
        .map_err(|error| error.to_string())?;
    fs::write(workdir.join("tracked.txt"), b"lane-only\n").map_err(|error| error.to_string())?;
    let (comparison, _) = db
        .compare_materialized_lane_candidates("ledger-lane", CandidateMaterialization::ManifestOnly)
        .map_err(|error| format!("lane authoritative comparison: {error}"))?;
    let lane_hash = hex::encode(Sha256::digest(b"lane-only\n"));
    let observed = comparison
        .disk_manifest
        .get("tracked.txt")
        .map(|entry| entry.content_hash.as_str());
    if observed != Some(lane_hash.as_str()) {
        return Err(format!(
            "lane snapshot used the wrong pinned root: expected {lane_hash}, got {observed:?}"
        ));
    }

    // Missing marker is reconciliation input, never a clean shortcut.
    fs::remove_file(workdir.join(".trail/workdir-manifest.json"))
        .map_err(|error| error.to_string())?;
    let (after_missing_marker, _) = db
        .compare_materialized_lane_candidates("ledger-lane", CandidateMaterialization::ManifestOnly)
        .map_err(|error| format!("missing-marker reconciliation: {error}"))?;
    if after_missing_marker
        .disk_manifest
        .get("tracked.txt")
        .map(|entry| entry.content_hash.as_str())
        != Some(lane_hash.as_str())
    {
        return Err("missing-marker reconciliation lost the lane candidate".into());
    }

    // Forced sync implementations may replace the directory inode at the
    // registered path. The old watcher/tail must be discarded and a new epoch
    // reconciled against the replacement root before any clean decision.
    let replaced = workdir.with_extension("replaced-root");
    fs::rename(&workdir, &replaced).map_err(|error| error.to_string())?;
    fs::create_dir(&workdir).map_err(|error| error.to_string())?;
    fs::create_dir(workdir.join(".trail")).map_err(|error| error.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(workdir.join(".trail"), fs::Permissions::from_mode(0o700))
            .map_err(|error| error.to_string())?;
    }
    fs::write(workdir.join("tracked.txt"), b"lane-only\n").map_err(|error| error.to_string())?;
    let (after_root_replacement, _) = db
        .compare_materialized_lane_candidates("ledger-lane", CandidateMaterialization::ManifestOnly)
        .map_err(|error| format!("replacement-root reconciliation: {error}"))?;
    if after_root_replacement
        .disk_manifest
        .get("tracked.txt")
        .map(|entry| entry.content_hash.as_str())
        != Some(lane_hash.as_str())
    {
        return Err("replacement-root reconciliation used the retired watcher root".into());
    }
    Ok(())
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
    pub(crate) fn filter_controlled_lane_sparse_candidates(
        &self,
        lane: &str,
        policy: &super::CompiledPolicy,
        candidates: &mut CandidateSnapshot,
    ) -> crate::Result<Option<Vec<String>>> {
        let branch = self.lane_branch(lane)?;
        let workdir = std::path::PathBuf::from(branch.workdir.as_deref().ok_or_else(|| {
            crate::Error::InvalidInput(format!(
                "lane `{}` does not have a materialized workdir",
                branch.lane_id
            ))
        })?);
        let declared_sparse = self
            .lane_sparse_paths_from_metadata(&branch.lane_id)?
            .is_some_and(|paths| !paths.is_empty());
        let selection = self.authenticated_lane_sparse_selection(&workdir)?;
        if declared_sparse && selection.is_none() {
            return Err(crate::Error::ChangeLedgerReconcileRequired {
                scope: candidates.expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: "sparse lane selection metadata is missing".into(),
                command: format!("trail lane status {}", branch.lane_id),
            });
        }
        let Some(selected) = selection.as_ref() else {
            return Ok(selection);
        };
        let pinned = self.open_pinned_worktree_root(policy)?;
        if self.pinned_worktree_root_identity(&pinned) != candidates.expected.filesystem_identity {
            return Err(crate::Error::ChangeLedgerReconcileRequired {
                scope: candidates.expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: "sparse lane root identity changed before candidate filtering".into(),
                command: format!("trail lane status {}", branch.lane_id),
            });
        }
        let mut exact_paths = Vec::new();
        for path in std::mem::take(&mut candidates.exact_paths) {
            if sparse_selection_intersects(selected, path.as_str())
                || self.pinned_worktree_path_is_visible(&pinned, path.as_str())?
            {
                exact_paths.push(path);
            }
        }
        candidates.exact_paths = exact_paths;
        let mut prefixes = Vec::new();
        for prefix in std::mem::take(&mut candidates.prefixes) {
            if sparse_selection_intersects(selected, prefix.path.as_str())
                || self.pinned_worktree_path_is_visible(&pinned, prefix.path.as_str())?
            {
                prefixes.push(prefix);
            }
        }
        candidates.prefixes = prefixes;
        if !self.verify_pinned_worktree_root(&pinned)? {
            return Err(crate::Error::ChangeLedgerReconcileRequired {
                scope: candidates.expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: "sparse lane root identity changed during candidate filtering".into(),
                command: format!("trail lane status {}", branch.lane_id),
            });
        }
        Ok(selection)
    }

    pub(crate) fn compare_materialized_lane_candidates(
        &self,
        lane: &str,
        materialization: CandidateMaterialization,
    ) -> crate::Result<(CandidateComparison, FencedCandidateSnapshot)> {
        let branch = self.lane_branch(lane)?;
        let workdir = branch.workdir.as_deref().ok_or_else(|| {
            crate::Error::InvalidInput(format!(
                "lane `{}` does not have a materialized workdir",
                branch.lane_id
            ))
        })?;
        let workdir = std::path::PathBuf::from(workdir);
        let declared_sparse = self
            .lane_sparse_paths_from_metadata(&branch.lane_id)?
            .is_some_and(|paths| !paths.is_empty());
        let selection_used = std::cell::RefCell::new(None::<Option<Vec<String>>>);
        let result =
            self.with_materialized_lane_authoritative_snapshot(lane, |db, policy, candidates| {
                let sparse_selection = db.authenticated_lane_sparse_selection(&workdir)?;
                if declared_sparse && sparse_selection.is_none() {
                    return Err(crate::Error::ChangeLedgerReconcileRequired {
                        scope: candidates.expected.scope_id.to_text(),
                        state: "untrusted_gap".into(),
                        reason: "sparse lane selection metadata is missing".into(),
                        command: format!("trail lane status {}", branch.lane_id),
                    });
                }
                selection_used.replace(Some(sparse_selection.clone()));
                let mut candidates = candidates.clone();
                if let Some(selection) = sparse_selection.as_ref() {
                    // Full reconciliation deliberately notices baseline files that
                    // are absent on disk. In a sparse lane, absence outside the
                    // authenticated materialized selection is not a deletion. Keep
                    // selected/intersecting paths plus any currently visible path
                    // (a newly created file outside the selection), and discard
                    // only baseline-only absences.
                    let pinned = db.open_pinned_worktree_root(policy)?;
                    if db.pinned_worktree_root_identity(&pinned)
                        != candidates.expected.filesystem_identity
                    {
                        return Err(crate::Error::ChangeLedgerReconcileRequired {
                            scope: candidates.expected.scope_id.to_text(),
                            state: "untrusted_gap".into(),
                            reason: "sparse lane root identity changed before candidate filtering"
                                .into(),
                            command: format!("trail lane status {}", branch.lane_id),
                        });
                    }
                    let mut exact_paths = Vec::new();
                    for path in std::mem::take(&mut candidates.exact_paths) {
                        if sparse_selection_intersects(selection, path.as_str())
                            || db.pinned_worktree_path_is_visible(&pinned, path.as_str())?
                        {
                            exact_paths.push(path);
                        }
                    }
                    candidates.exact_paths = exact_paths;
                    let mut prefixes = Vec::new();
                    for prefix in std::mem::take(&mut candidates.prefixes) {
                        if sparse_selection_intersects(selection, prefix.path.as_str())
                            || db.pinned_worktree_path_is_visible(&pinned, prefix.path.as_str())?
                        {
                            prefixes.push(prefix);
                        }
                    }
                    candidates.prefixes = prefixes;
                    if !db.verify_pinned_worktree_root(&pinned)? {
                        return Err(crate::Error::ChangeLedgerReconcileRequired {
                            scope: candidates.expected.scope_id.to_text(),
                            state: "untrusted_gap".into(),
                            reason: "sparse lane root identity changed during candidate filtering"
                                .into(),
                            command: format!("trail lane status {}", branch.lane_id),
                        });
                    }
                    candidates.acknowledgement_tokens.retain(|token| {
                        candidates
                            .exact_paths
                            .iter()
                            .any(|path| path == &token.path)
                            || candidates
                                .prefixes
                                .iter()
                                .any(|prefix| prefix.path == token.path)
                    });
                }
                db.compare_authoritative_candidates(
                    policy,
                    &candidates,
                    &candidates.expected.baseline_root,
                    materialization,
                )
            })?;
        let current_selection = self.authenticated_lane_sparse_selection(&workdir)?;
        if selection_used.into_inner() != Some(current_selection) {
            return Err(crate::Error::ChangeLedgerReconcileRequired {
                scope: result.1.candidates.expected.scope_id.to_text(),
                state: "stale_baseline".into(),
                reason: "sparse lane selection changed during authoritative snapshot".into(),
                command: format!("trail lane status {}", branch.lane_id),
            });
        }
        Ok(result)
    }

    pub(crate) fn with_workspace_authoritative_snapshot<T, F>(
        &self,
        consume: F,
    ) -> crate::Result<(T, FencedCandidateSnapshot)>
    where
        F: FnMut(&crate::Trail, &super::CompiledPolicy, &CandidateSnapshot) -> crate::Result<T>,
    {
        let mut runtime = self
            .changed_path_daemon_registry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .workspace
            .take()
            .ok_or_else(|| {
                crate::Error::DaemonUnavailable(
                    "changed-path observer runtime is unavailable".into(),
                )
            })?;
        let result = runtime.with_authoritative_snapshot(self, consume);
        self.changed_path_daemon_registry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .workspace = Some(runtime);
        result
    }

    /// Consume an exact c1 -> compare -> c2 snapshot rooted at one qualified
    /// materialized lane. Missing, stale, or structurally invalid marker state
    /// is never interpreted as clean: it forces a full lane reconciliation and
    /// a newly bound marker before the candidate set may be consumed.
    pub(crate) fn with_materialized_lane_authoritative_snapshot<T, F>(
        &self,
        lane: &str,
        consume: F,
    ) -> crate::Result<(T, FencedCandidateSnapshot)>
    where
        F: FnMut(&crate::Trail, &super::CompiledPolicy, &CandidateSnapshot) -> crate::Result<T>,
    {
        let lane_id = self.lane_branch(lane)?.lane_id;
        self.ensure_materialized_lane_snapshot_authority(&lane_id)?;
        let branch = self.lane_branch(&lane_id)?;
        let workdir = branch.workdir.as_deref().ok_or_else(|| {
            crate::Error::InvalidInput(format!(
                "lane `{}` does not have a materialized workdir",
                branch.lane_id
            ))
        })?;
        let sparse_selection_fingerprint =
            self.materialized_lane_sparse_selection_fingerprint(std::path::Path::new(workdir))?;
        let mut runtime = self
            .changed_path_daemon_registry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .materialized_lanes
            .remove(&lane_id)
            .ok_or_else(|| {
                crate::Error::DaemonUnavailable(format!(
                    "changed-path observer runtime for materialized lane `{lane_id}` is unavailable"
                ))
            })?;
        let result = runtime.with_authoritative_snapshot(self, consume);
        self.changed_path_daemon_registry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .materialized_lanes
            .insert(lane_id.clone(), runtime);
        // Keep the compact marker bound to c2. It remains only a restart hint;
        // every clean claim still authenticates a fresh native fence.
        // Publish the exact selection binding captured before c1. If the
        // selection changes concurrently, validation sees a fingerprint
        // mismatch and forces reconciliation instead of accepting false-clean
        // candidates for a different sparse view.
        let marker_result =
            self.publish_lane_marker_for_sparse_selection(&lane_id, sparse_selection_fingerprint);
        match (result, marker_result) {
            (Ok(snapshot), Ok(())) => Ok(snapshot),
            (Err(error), _) => Err(error),
            (_, Err(error)) => Err(error),
        }
    }

    fn ensure_materialized_lane_snapshot_authority(&self, lane: &str) -> crate::Result<()> {
        let branch = self.lane_branch(lane)?;
        let workdir = branch.workdir.as_deref().ok_or_else(|| {
            crate::Error::InvalidInput(format!(
                "lane `{}` does not have a materialized workdir",
                branch.lane_id
            ))
        })?;
        let head = self.get_ref(&branch.ref_name)?;
        let runtime_present = self
            .changed_path_daemon_registry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .materialized_lanes
            .contains_key(&branch.lane_id);
        let runtime_matches = runtime_present
            && super::materialized_lane_daemon_matches_target(self, &branch.lane_id)?;
        if runtime_present && !runtime_matches {
            // A forced sync may atomically replace the root inode. Drop the old
            // watcher before starting a new epoch against the replacement root;
            // reusing its retained tail would falsely bridge two filesystems.
            self.changed_path_daemon_registry
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .materialized_lanes
                .remove(&branch.lane_id);
        }
        if !runtime_matches {
            if runtime_present {
                super::daemon::prepare_materialized_lane_daemon_verified_replacement(
                    self,
                    &branch.lane_id,
                )?;
            } else {
                super::prepare_materialized_lane_daemon(self, &branch.lane_id, true)?;
            }
        }
        let valid = self
            .validated_materialized_lane_marker_v2(&branch, std::path::Path::new(workdir), &head)?
            .is_some();
        if valid {
            // Authenticate continuity and the current sidecar/owner chain. A
            // persisted SQL `trusted` bit or marker alone is never clean
            // authority.
            if let Err(error) = super::materialized_lane_daemon_fence(self, &branch.lane_id) {
                if !matches!(error, crate::Error::ChangeLedgerReconcileRequired { .. }) {
                    return Err(error);
                }
                if let Err(error) = super::materialized_lane_daemon_reconcile(
                    self,
                    &branch.lane_id,
                    "materialized_lane_authority_recovery",
                ) {
                    if matches!(
                        error,
                        crate::Error::DaemonUnavailable(_)
                            | crate::Error::ChangeLedgerReconcileRequired { .. }
                    ) {
                        self.restart_verified_materialized_lane_runtime(&branch.lane_id)?;
                    } else {
                        return Err(error);
                    }
                }
            }
        } else {
            if let Err(error) = super::materialized_lane_daemon_reconcile(
                self,
                &branch.lane_id,
                "materialized_lane_marker_reconciliation",
            ) {
                if matches!(
                    error,
                    crate::Error::DaemonUnavailable(_)
                        | crate::Error::ChangeLedgerReconcileRequired { .. }
                ) {
                    self.restart_verified_materialized_lane_runtime(&branch.lane_id)?;
                } else {
                    return Err(error);
                }
            }
            // Reconciliation establishes the authoritative cut; publish its
            // compact marker only after the runtime has been returned to the
            // registry so the marker can bind the exact segment ID and cut.
            self.publish_lane_marker_if_materialized(&branch.lane_id)?;
            let branch = self.lane_branch(&branch.lane_id)?;
            let head = self.get_ref(&branch.ref_name)?;
            let workdir = branch.workdir.as_deref().ok_or_else(|| {
                crate::Error::InvalidInput(format!(
                    "lane `{}` lost its materialized workdir during reconciliation",
                    branch.lane_id
                ))
            })?;
            if self
                .validated_materialized_lane_marker_v2(
                    &branch,
                    std::path::Path::new(workdir),
                    &head,
                )?
                .is_none()
            {
                return Err(crate::Error::ChangeLedgerReconcileRequired {
                    scope: super::materialized_lane_scope_id(
                        &self.config.workspace.id.0,
                        &branch.lane_id,
                    )
                    .to_text(),
                    state: "untrusted_gap".into(),
                    reason:
                        "materialized lane marker remained unqualified after full reconciliation"
                            .into(),
                    command: format!("trail lane status {}", branch.lane_id),
                });
            }
        }
        Ok(())
    }

    fn restart_verified_materialized_lane_runtime(&self, lane: &str) -> crate::Result<()> {
        self.changed_path_daemon_registry
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .materialized_lanes
            .remove(lane);
        super::daemon::prepare_materialized_lane_daemon_verified_replacement(self, lane)?;
        Ok(())
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
        )
        .map_err(|error| crate::Error::CommittedRepairRequired {
            operation: operation_id.0.clone(),
            repair: "ref mirror".into(),
            reason: error.to_string(),
        })?;
        let target = BaselineIdentity {
            ref_name: expected_ref.name.clone(),
            ref_generation: u64::try_from(expected_ref.generation + 1)
                .map_err(|_| crate::Error::InvalidInput("ref generation overflow".into()))?,
            change_id: operation.change_id.clone(),
            root_id: operation.after_root.clone(),
        };
        // Materialized-lane runtime repair is wrapper-owned so a committed
        // observed operation has exactly one post-commit transition. Workspace
        // observed recording has no wrapper repair closure and remains local.
        if lane_id.is_none() {
            if let Some(runtime) = self
                .changed_path_daemon_registry
                .lock()
                .unwrap_or_else(|poison| poison.into_inner())
                .workspace
                .as_mut()
            {
                runtime
                    .accept_observed_baseline(&observed.expected, &target)
                    .map_err(|error| crate::Error::CommittedRepairRequired {
                        operation: operation_id.0.clone(),
                        repair: "workspace observer runtime".into(),
                        reason: error.to_string(),
                    })?;
            }
        }
        Ok(operation_id)
    }
}
