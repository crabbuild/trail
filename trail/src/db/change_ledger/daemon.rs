use std::collections::HashMap;
use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::Duration;

use getrandom::getrandom;
use rusqlite::{named_params, params, OptionalExtension, Transaction, TransactionBehavior};
use sha2::{Digest, Sha256};

use super::FencedCandidateSnapshot;
use super::{
    compile_policy, fold_observer_interval, raw_event_invalidates_policy, reconcile_full_with_tail,
    revalidate_compiled_policy, BaselineIdentity, CompiledPolicy, DaemonLaunchBinding, DurableCut,
    EvidenceCut, EvidenceSource, ExpectedScope, FilesystemIdentity, IntentEvidence, IntentId,
    ObserverEvent, ObserverFence, ObserverLease, PersistedLogLimits, PolicyCompileContext,
    PolicyDependencyMetrics, PolicyIdentity, ProviderCapabilities, ProviderIdentity,
    QualifiedFilesystemProof, QualifiedObserver, RecoveredTail, RecoveryScope, ScopeId,
    ScopeIdentity, ScopeKind, SegmentWriter,
};
use crate::error::{Error, Result};
use crate::Trail;

type ControlledProjectionScopeRow = (
    String,
    String,
    String,
    String,
    String,
    Option<Vec<u8>>,
    i64,
    i64,
    i64,
);
type ObserverSegmentStateRow = (i64, i64, Option<String>, Option<String>, String, String);

pub(crate) struct WorkspaceDaemonProof {
    pub(crate) scope_id: String,
    pub(crate) epoch: u64,
    pub(crate) observer_owner_token: String,
    pub(crate) daemon_launch_nonce: Option<String>,
    pub(crate) cut: EvidenceCut,
    pub(crate) reconcile_report: Option<crate::model::ChangeLedgerReconcileReport>,
}

#[derive(Clone, Debug)]
pub(crate) struct VerifiedStaleWorkspaceOwner {
    pub(crate) stale_pid: u32,
    pub(crate) process_start_identity: String,
    pub(crate) scope_id: String,
    pub(crate) epoch: u64,
    pub(crate) observer_owner_token: String,
    pub(crate) daemon_launch_nonce: String,
}

#[derive(Clone, Debug)]
pub(crate) struct PersistedWorkspaceDaemonOwner {
    pub(crate) stale_pid: u32,
    pub(crate) process_start_identity: String,
    pub(crate) scope_id: String,
    pub(crate) epoch: u64,
    pub(crate) observer_owner_token: String,
    pub(crate) daemon_launch_nonce: String,
}

#[derive(Clone, Debug)]
pub(crate) struct WorkspaceDaemonLaunchIdentity {
    pub(crate) nonce: String,
    pub(crate) pid: u32,
    pub(crate) process_start_identity: String,
}

#[derive(Default)]
pub(crate) struct ChangedPathDaemonRegistry {
    pub(super) workspace: Option<WorkspaceDaemonRuntime>,
    pub(super) materialized_lanes: HashMap<String, WorkspaceDaemonRuntime>,
}

struct DaemonScopeTarget {
    root: std::path::PathBuf,
    identity: ScopeIdentity,
    baseline: BaselineIdentity,
}

const MAX_STARTUP_POLICY_RETRIES: usize = 2;

#[cfg(test)]
type WorkspaceRetryBoundaryHook = Box<dyn FnOnce(&Trail) -> Result<()> + Send>;
#[cfg(test)]
static WORKSPACE_RETRY_BOUNDARY_HOOK: std::sync::OnceLock<
    std::sync::Mutex<HashMap<ScopeId, WorkspaceRetryBoundaryHook>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
fn install_workspace_retry_boundary_hook(
    scope_id: ScopeId,
    hook: impl FnOnce(&Trail) -> Result<()> + Send + 'static,
) {
    WORKSPACE_RETRY_BOUNDARY_HOOK
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .insert(scope_id, Box::new(hook));
}

#[cfg(test)]
fn run_workspace_retry_boundary_hook(db: &Trail) -> Result<()> {
    let scope_id = workspace_daemon_target(db)?.identity.scope_id;
    let hook = WORKSPACE_RETRY_BOUNDARY_HOOK
        .get_or_init(|| std::sync::Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .remove(&scope_id);
    match hook {
        Some(hook) => hook(db),
        None => Ok(()),
    }
}

pub(crate) fn prepare_workspace_daemon(
    db: &Trail,
    replace_stale_owner: bool,
) -> Result<WorkspaceDaemonProof> {
    prepare_workspace_daemon_inner(db, replace_stale_owner, None, None)
}

pub(crate) fn prepare_workspace_daemon_verified_replacement(
    db: &Trail,
    verified_stale_owner: VerifiedStaleWorkspaceOwner,
    launch: Option<WorkspaceDaemonLaunchIdentity>,
) -> Result<WorkspaceDaemonProof> {
    prepare_workspace_daemon_inner(db, true, Some(verified_stale_owner), launch)
}

fn prepare_workspace_daemon_inner(
    db: &Trail,
    replace_stale_owner: bool,
    verified_stale_owner: Option<VerifiedStaleWorkspaceOwner>,
    launch: Option<WorkspaceDaemonLaunchIdentity>,
) -> Result<WorkspaceDaemonProof> {
    if daemon_registry(db).workspace.is_some() {
        return workspace_daemon_fence(db, None, None);
    }
    let mut retries = 0_usize;
    let mut verified_stale_owner = verified_stale_owner;
    loop {
        let mut runtime = match WorkspaceDaemonRuntime::start(
            db,
            workspace_daemon_target(db)?,
            replace_stale_owner || retries > 0,
            verified_stale_owner.is_none(),
            verified_stale_owner.as_ref(),
            launch.as_ref(),
        ) {
            Ok(runtime) => runtime,
            Err(error)
                if startup_policy_retryable(&error) && retries < MAX_STARTUP_POLICY_RETRIES =>
            {
                verified_stale_owner = workspace_retry_owner_capability(db, launch.as_ref())?;
                retries += 1;
                #[cfg(test)]
                run_workspace_retry_boundary_hook(db)?;
                startup_policy_retry_delay(retries);
                continue;
            }
            Err(error) => return Err(error),
        };
        match runtime.reconcile(db, "daemon_initial_full_reconciliation") {
            Ok(proof) => {
                daemon_registry(db).workspace = Some(runtime);
                return Ok(proof);
            }
            Err(error)
                if startup_policy_retryable(&error) && retries < MAX_STARTUP_POLICY_RETRIES =>
            {
                verified_stale_owner = workspace_retry_owner_capability(db, launch.as_ref())?;
                drop(runtime);
                retries += 1;
                #[cfg(test)]
                run_workspace_retry_boundary_hook(db)?;
                startup_policy_retry_delay(retries);
            }
            Err(error) => return Err(error),
        }
    }
}

fn workspace_retry_owner_capability(
    db: &Trail,
    launch: Option<&WorkspaceDaemonLaunchIdentity>,
) -> Result<Option<VerifiedStaleWorkspaceOwner>> {
    let Some(launch) = launch else {
        return Ok(None);
    };
    verified_stale_workspace_owner_for_launch(
        db,
        launch.pid,
        &launch.process_start_identity,
        &launch.nonce,
    )
}

pub(crate) fn prepare_workspace_daemon_launch(
    db: &Trail,
    launch: WorkspaceDaemonLaunchIdentity,
    verified_stale_owner: Option<VerifiedStaleWorkspaceOwner>,
) -> Result<WorkspaceDaemonProof> {
    prepare_workspace_daemon_inner(
        db,
        verified_stale_owner.is_some(),
        verified_stale_owner,
        Some(launch),
    )
}

pub(crate) fn verified_stale_workspace_owner_for_launch(
    db: &Trail,
    stale_pid: u32,
    process_start_identity: &str,
    daemon_launch_nonce: &str,
) -> Result<Option<VerifiedStaleWorkspaceOwner>> {
    if stale_pid == 0 || process_start_identity.is_empty() || daemon_launch_nonce.len() != 64 {
        return Err(Error::DaemonUnavailable(
            "stale workspace daemon launch identity is malformed".into(),
        ));
    }
    let binding = db
        .conn
        .query_row(
            "SELECT scope_id,epoch,owner_token,daemon_pid,daemon_process_start_identity
             FROM changed_path_observer_owners
             WHERE daemon_launch_nonce=?1",
            params![daemon_launch_nonce],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )
        .optional()?;
    let Some((scope_id, epoch, owner_token, bound_pid, bound_start)) = binding else {
        return Ok(None);
    };
    if bound_pid != i64::from(stale_pid) || bound_start != process_start_identity {
        return Err(Error::DaemonUnavailable(
            "stale workspace daemon publication does not match its atomically persisted launch identity"
                .into(),
        ));
    }
    let epoch = u64::try_from(epoch)
        .map_err(|_| Error::Corrupt("negative stale daemon owner epoch".into()))?;
    Ok(Some(VerifiedStaleWorkspaceOwner {
        stale_pid,
        process_start_identity: process_start_identity.to_string(),
        scope_id,
        epoch,
        observer_owner_token: owner_token,
        daemon_launch_nonce: daemon_launch_nonce.to_string(),
    }))
}

pub(crate) fn persisted_workspace_daemon_owner(
    db: &Trail,
) -> Result<Option<PersistedWorkspaceDaemonOwner>> {
    let scope_id = workspace_daemon_target(db)?.identity.scope_id;
    let Some(stored) = load_existing_scope(db, scope_id)? else {
        return Ok(None);
    };
    let Some(owner) = stored.observer_owner else {
        if stored.observer_owner_token.is_some() {
            return Err(daemon_owner_authority_inconsistent(scope_id));
        }
        return Ok(None);
    };
    if stored.scope_kind != ScopeKind::Workspace.as_str()
        || owner.epoch != stored.epoch
        || stored.observer_owner_token.as_deref() != Some(owner.owner_token.as_str())
    {
        return Err(daemon_owner_authority_inconsistent(scope_id));
    }
    let (Some(daemon_launch_nonce), Some(daemon_pid), Some(process_start_identity)) = (
        owner.daemon_launch_nonce,
        owner.daemon_pid,
        owner.daemon_process_start_identity,
    ) else {
        return Err(Error::DaemonUnavailable(
            "persisted workspace daemon owner lacks an exact launch binding".into(),
        ));
    };
    let stale_pid = u32::try_from(daemon_pid).map_err(|_| {
        Error::DaemonUnavailable(
            "persisted workspace daemon owner launch binding is malformed".into(),
        )
    })?;
    if stale_pid == 0
        || stale_pid > i32::MAX as u32
        || process_start_identity.is_empty()
        || !is_canonical_authority_token(&daemon_launch_nonce)
        || !is_canonical_authority_token(&owner.owner_token)
    {
        return Err(Error::DaemonUnavailable(
            "persisted workspace daemon owner launch binding is malformed".into(),
        ));
    }
    Ok(Some(PersistedWorkspaceDaemonOwner {
        stale_pid,
        process_start_identity,
        scope_id: scope_id.to_text(),
        epoch: stored.epoch,
        observer_owner_token: owner.owner_token,
        daemon_launch_nonce,
    }))
}

fn is_canonical_authority_token(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

pub(crate) fn prepare_workspace_controlled_projection(db: &mut Trail) -> Result<ExpectedScope> {
    prepare_workspace_daemon(db, true)?;
    with_workspace_runtime(db, |runtime, db| {
        runtime.fence_and_seal(db)?;
        Ok(runtime.expected.clone())
    })
}

pub(crate) fn workspace_daemon_fence(
    db: &Trail,
    scope_id: Option<&str>,
    epoch: Option<u64>,
) -> Result<WorkspaceDaemonProof> {
    let mut runtime = daemon_registry(db).workspace.take().ok_or_else(|| {
        Error::DaemonUnavailable("changed-path observer runtime is unavailable".into())
    })?;
    runtime.validate_request(scope_id, epoch)?;
    let result = runtime.fence(db);
    match result {
        Ok(proof) => {
            daemon_registry(db).workspace = Some(runtime);
            Ok(proof)
        }
        Err(error) if startup_policy_retryable(&error) => {
            // A fenced policy dependency changed outside native observer
            // coverage. The failed observer has already revoked its durable
            // owner; discard it, recompile policy, and establish a fresh full
            // reconciliation before continuing the command.
            drop(runtime);
            prepare_workspace_daemon_inner(db, true, None, None)
        }
        Err(error) => {
            daemon_registry(db).workspace = Some(runtime);
            Err(error)
        }
    }
}

pub(crate) fn workspace_daemon_reconcile(
    db: &Trail,
    scope_id: Option<&str>,
    epoch: Option<u64>,
) -> Result<WorkspaceDaemonProof> {
    let mut runtime = daemon_registry(db).workspace.take().ok_or_else(|| {
        Error::DaemonUnavailable("changed-path observer runtime is unavailable".into())
    })?;
    runtime.validate_request(scope_id, epoch)?;
    let result = runtime.reconcile(db, "daemon_requested_full_reconciliation");
    daemon_registry(db).workspace = Some(runtime);
    result
}

pub(crate) fn workspace_daemon_full_reconcile(
    db: &Trail,
) -> Result<crate::model::ChangeLedgerReconcileReport> {
    let running = daemon_registry(db).workspace.is_some();
    let proof = if running {
        workspace_daemon_reconcile(db, None, None)?
    } else {
        prepare_workspace_daemon(db, true)?
    };
    proof.reconcile_report.ok_or_else(|| {
        Error::DaemonUnavailable("workspace daemon returned no reconciliation report".into())
    })
}

pub(crate) fn workspace_daemon_ready_proof(db: &Trail) -> Result<WorkspaceDaemonProof> {
    daemon_registry(db)
        .workspace
        .as_ref()
        .ok_or_else(|| {
            Error::DaemonUnavailable("changed-path observer runtime is unavailable".into())
        })?
        .current_proof()
}

pub(crate) fn prepare_materialized_lane_daemon(
    db: &Trail,
    lane: &str,
    replace_verified_stale_owner: bool,
) -> Result<WorkspaceDaemonProof> {
    prepare_materialized_lane_daemon_inner(db, lane, replace_verified_stale_owner, true, true)
}

pub(super) fn prepare_materialized_lane_daemon_verified_replacement(
    db: &Trail,
    lane: &str,
) -> Result<WorkspaceDaemonProof> {
    prepare_materialized_lane_daemon_inner(db, lane, true, false, false)
}

fn prepare_materialized_lane_daemon_inner(
    db: &Trail,
    lane: &str,
    replace_verified_stale_owner: bool,
    refuse_live_unverified_owner: bool,
    publish_marker: bool,
) -> Result<WorkspaceDaemonProof> {
    let lane_id = db.lane_branch(lane)?.lane_id;
    if daemon_registry(db)
        .materialized_lanes
        .contains_key(&lane_id)
    {
        let proof = with_materialized_lane_runtime(db, &lane_id, |runtime, db| runtime.fence(db))?;
        if publish_marker {
            db.publish_lane_marker_if_materialized(&lane_id)?;
        }
        return Ok(proof);
    }
    let mut retries = 0_usize;
    loop {
        let target = materialized_lane_daemon_target(db, &lane_id)?;
        let mut runtime = match WorkspaceDaemonRuntime::start(
            db,
            target,
            replace_verified_stale_owner,
            refuse_live_unverified_owner,
            None,
            None,
        ) {
            Ok(runtime) => runtime,
            Err(error)
                if startup_policy_retryable(&error) && retries < MAX_STARTUP_POLICY_RETRIES =>
            {
                retries += 1;
                startup_policy_retry_delay(retries);
                continue;
            }
            Err(error @ Error::ChangeLedgerReconcileRequired { .. }) => return Err(error),
            Err(error) => {
                return Err(Error::DaemonUnavailable(format!(
                    "materialized-lane observer startup failed: {error}"
                )))
            }
        };
        match runtime.reconcile(db, "materialized_lane_initial_full_reconciliation") {
            Ok(proof) => {
                daemon_registry(db)
                    .materialized_lanes
                    .insert(lane_id.clone(), runtime);
                if publish_marker {
                    db.publish_lane_marker_if_materialized(&lane_id)?;
                }
                return Ok(proof);
            }
            Err(error)
                if startup_policy_retryable(&error) && retries < MAX_STARTUP_POLICY_RETRIES =>
            {
                drop(runtime);
                retries += 1;
                startup_policy_retry_delay(retries);
            }
            Err(error @ Error::ChangeLedgerReconcileRequired { .. }) => return Err(error),
            Err(error) => {
                return Err(Error::DaemonUnavailable(format!(
                    "materialized-lane initial full reconciliation failed: {error}"
                )))
            }
        }
    }
}

fn startup_policy_retryable(error: &Error) -> bool {
    match error {
        Error::ChangeLedgerReconcileRequired { reason, .. } => {
            reason.contains("policy_dependency_invalidated")
                || reason.contains("recording policy changed")
        }
        Error::DaemonUnavailable(reason) => {
            reason.contains("recording policy changed during observer startup")
        }
        _ => false,
    }
}

pub(crate) fn policy_runtime_restart_required(error: &Error) -> bool {
    startup_policy_retryable(error)
}

fn startup_policy_retry_delay(retry: usize) {
    let millis = if retry <= 1 { 100 } else { 400 };
    std::thread::sleep(Duration::from_millis(millis));
}

pub(crate) fn materialized_lane_daemon_fence(
    db: &Trail,
    lane: &str,
) -> Result<WorkspaceDaemonProof> {
    let proof = with_materialized_lane_runtime(db, lane, |runtime, db| runtime.fence(db))?;
    db.publish_lane_marker_if_materialized(lane)?;
    Ok(proof)
}

/// Establish an exact sidecar boundary immediately before preparing a
/// controlled materialized-lane intent.  The returned scope's provider cursor
/// is the start cursor of the newly-opened segment, so the subsequent intent
/// cannot begin ambiguously in the middle of an observer segment.
pub(crate) fn prepare_materialized_lane_controlled_projection(
    db: &mut Trail,
    lane: &str,
) -> Result<ExpectedScope> {
    // Controlled preparation must not write the marker inside the watched
    // workdir. Such a write is native observer traffic in the fence-to-seal
    // gap. The projection protocol repairs the marker after its terminal SQL
    // publication through its existing repair callback.
    let lane_id = db.lane_branch(lane)?.lane_id;
    let mut reconciled = false;
    loop {
        prepare_materialized_lane_daemon_verified_replacement(db, &lane_id)?;
        match with_materialized_lane_runtime(db, &lane_id, |runtime, db| {
            runtime.fence_and_seal(db)?;
            Ok(runtime.expected.clone())
        }) {
            Ok(expected) => return Ok(expected),
            Err(error)
                if !reconciled
                    && requires_reconciliation(&error)
                    && !startup_policy_retryable(&error) =>
            {
                daemon_registry(db).materialized_lanes.remove(&lane_id);
                prepare_materialized_lane_daemon(db, &lane_id, true)?;
                reconciled = true;
            }
            Err(error) => return Err(error),
        }
    }
}

pub(crate) fn materialized_lane_daemon_matches_target(db: &Trail, lane: &str) -> Result<bool> {
    let lane_id = db.lane_branch(lane)?.lane_id;
    let registry = daemon_registry(db);
    let Some(runtime) = registry.materialized_lanes.get(&lane_id) else {
        return Ok(false);
    };
    runtime.matches_target(&materialized_lane_daemon_target(db, &lane_id)?)
}

pub(crate) fn materialized_lane_daemon_reconcile(
    db: &Trail,
    lane: &str,
    reason: &str,
) -> Result<WorkspaceDaemonProof> {
    let proof =
        with_materialized_lane_runtime(db, lane, |runtime, db| runtime.reconcile(db, reason))?;
    db.publish_lane_marker_if_materialized(lane)?;
    Ok(proof)
}

pub(crate) fn materialized_lane_daemon_full_reconcile(
    db: &Trail,
    lane: &str,
) -> Result<crate::model::ChangeLedgerReconcileReport> {
    let lane_id = db.lane_branch(lane)?.lane_id;
    let running = daemon_registry(db)
        .materialized_lanes
        .contains_key(&lane_id);
    let proof = if running {
        materialized_lane_daemon_reconcile(db, &lane_id, "user_requested_full_reconciliation")?
    } else {
        prepare_materialized_lane_daemon(db, &lane_id, true)?
    };
    proof.reconcile_report.ok_or_else(|| {
        Error::DaemonUnavailable(
            "materialized-lane daemon returned no reconciliation report".into(),
        )
    })
}

pub(crate) fn accept_materialized_lane_daemon_baseline(
    db: &Trail,
    lane: &str,
    expected: &ExpectedScope,
    target: &BaselineIdentity,
) -> Result<()> {
    with_materialized_lane_runtime(db, lane, |runtime, _| {
        runtime.accept_observed_baseline(expected, target)
    })?;
    db.publish_lane_marker_if_materialized(lane)
}

/// Run one controlled materialized-lane filesystem interval. The intent must
/// already be durably prepared at the runtime's retained cut. `apply_and_sync`
/// mutates and durably syncs bytes; c1 is then fenced before `pinned_verify`
/// compares the intended paths through descriptor-relative reads. A final c2
/// retains every post-c1 race and the returned sidecar-backed proof is suitable
/// for `mark_filesystem_applied`.
pub(crate) fn with_materialized_lane_controlled_interval<A, V>(
    db: &mut Trail,
    lane: &str,
    intent_id: &IntentId,
    evidence: &IntentEvidence,
    apply_and_sync: A,
    pinned_verify: V,
) -> Result<QualifiedFilesystemProof>
where
    A: FnOnce(&mut Trail) -> Result<()>,
    V: FnOnce(&mut Trail, &CompiledPolicy, &super::CandidateSnapshot) -> Result<()>,
{
    let lane_id = db.lane_branch(lane)?.lane_id;
    with_controlled_interval(
        db,
        ControlledRuntimeKey::MaterializedLane(lane_id),
        intent_id,
        evidence,
        apply_and_sync,
        pinned_verify,
    )
}

pub(crate) fn with_workspace_controlled_interval<A, V>(
    db: &mut Trail,
    intent_id: &IntentId,
    evidence: &IntentEvidence,
    apply_and_sync: A,
    pinned_verify: V,
) -> Result<QualifiedFilesystemProof>
where
    A: FnOnce(&mut Trail) -> Result<()>,
    V: FnOnce(&mut Trail, &CompiledPolicy, &super::CandidateSnapshot) -> Result<()>,
{
    with_controlled_interval(
        db,
        ControlledRuntimeKey::Workspace,
        intent_id,
        evidence,
        apply_and_sync,
        pinned_verify,
    )
}

#[derive(Clone)]
enum ControlledRuntimeKey {
    Workspace,
    MaterializedLane(String),
}

fn with_controlled_interval<A, V>(
    db: &mut Trail,
    runtime_key: ControlledRuntimeKey,
    intent_id: &IntentId,
    evidence: &IntentEvidence,
    apply_and_sync: A,
    pinned_verify: V,
) -> Result<QualifiedFilesystemProof>
where
    A: FnOnce(&mut Trail) -> Result<()>,
    V: FnOnce(&mut Trail, &CompiledPolicy, &super::CandidateSnapshot) -> Result<()>,
{
    // Establish/recover the exact lane scope before inspecting the prepared
    // intent. A no-op snapshot supplies the authenticated retained c1 cut; it
    // must happen before prepare_intent, so callers normally invoke this only
    // after an earlier status/producer preparation fence.
    ensure_controlled_runtime(db, &runtime_key)?;
    let command = controlled_status_command(&runtime_key);
    let expected = {
        let registry = daemon_registry(db);
        controlled_runtime(&registry, &runtime_key)
            .ok_or_else(|| Error::DaemonUnavailable("lane runtime disappeared".into()))?
            .expected
            .clone()
    };
    let (intent_scope, expected_root_id, start_cursor): (String, String, Option<Vec<u8>>) =
        db.conn.query_row(
            "SELECT scope_id,expected_root_id,start_cursor FROM changed_path_intents
             WHERE intent_id=?1 AND lifecycle_state='prepared'",
            [&intent_id.0],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )?;
    if intent_scope != expected.scope_id.to_text() {
        return Err(Error::Conflict(format!(
            "intent `{}` belongs to a different changed-path scope",
            intent_id.0
        )));
    }
    apply_and_sync(db)?;
    if !controlled_runtime_matches_target(db, &runtime_key)? {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "stale_baseline".into(),
            reason: "controlled lane projection replaced or rebound the pinned root".into(),
            command: command.clone(),
        });
    }
    // c1 closes the controlled write interval before any comparison. An
    // external same-path write after this fence can never be acknowledged by
    // the intent, even if it races the pinned verification below.
    let c1 =
        with_controlled_runtime(db, &runtime_key, |runtime, db| runtime.fence_and_seal(db))?.cut;
    let (policy, mut candidates) = {
        let registry = daemon_registry(db);
        let runtime = controlled_runtime(&registry, &runtime_key)
            .ok_or_else(|| Error::DaemonUnavailable("lane runtime disappeared".into()))?;
        (
            runtime.policy.clone(),
            db.changed_path_ledger()
                .snapshot_candidates_for_controlled_intent(
                    &runtime.expected,
                    &intent_id.0,
                    start_cursor.as_deref(),
                )?,
        )
    };
    candidates.cut = c1.clone();
    db.filter_controlled_internal_candidates(&policy, &mut candidates, evidence)?;
    let sparse_selection = match &runtime_key {
        ControlledRuntimeKey::Workspace => None,
        ControlledRuntimeKey::MaterializedLane(lane) => {
            db.filter_controlled_lane_sparse_candidates(lane, &policy, &mut candidates)?
        }
    };
    pinned_verify(db, &policy, &candidates)?;
    if !controlled_runtime_matches_target(db, &runtime_key)? {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "stale_baseline".into(),
            reason: "controlled lane verification lost its pinned root".into(),
            command: command.clone(),
        });
    }
    // c2 closes the post-c1 comparison interval without another rotation.
    // c1 already sealed the controlled segment and advanced to a fresh
    // anchor; this ordinary fence leaves the scope exactly at publication_cut
    // while every event in (c1,c2] remains pending.
    let (c2, publication_durable) = with_controlled_runtime(db, &runtime_key, |runtime, db| {
        let proof = runtime.fence(db)?;
        let anchor = runtime.tail_anchor.as_ref().ok_or_else(|| {
            Error::DaemonUnavailable(
                "controlled runtime lost its authenticated publication anchor".into(),
            )
        })?;
        let durable = runtime
            .observer
            .authenticated_cut(&runtime.expected, anchor)?;
        if durable.last_sequence != proof.cut.sequence
            || durable.durable_end_offset != proof.cut.durable_offset
        {
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: runtime.expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: "publication fence lost its exact sidecar boundary".into(),
                command: controlled_status_command(&runtime_key),
            });
        }
        Ok((proof.cut, durable))
    })?;
    if let ControlledRuntimeKey::MaterializedLane(lane) = &runtime_key {
        let branch = db.lane_branch(lane)?;
        let workdir = std::path::PathBuf::from(branch.workdir.as_deref().ok_or_else(|| {
            Error::InvalidInput(format!(
                "lane `{}` does not have a materialized workdir",
                branch.lane_id
            ))
        })?);
        if db.authenticated_lane_sparse_selection(&workdir)? != sparse_selection {
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: expected.scope_id.to_text(),
                state: "stale_baseline".into(),
                reason: "sparse lane selection changed during controlled projection".into(),
                command: command.clone(),
            });
        }
    }

    let (
        scope_root_identity,
        filesystem_identity,
        provider_id,
        provider_identity,
        owner_token,
        fence_nonce,
        max_log_bytes,
        max_segment_bytes,
        max_tail_records,
    ): ControlledProjectionScopeRow = db.conn.query_row(
        "SELECT scope.scope_root_identity,scope.filesystem_identity,
                scope.provider_id,scope.provider_identity,owner.owner_token,owner.fence_nonce,
                scope.max_observer_log_bytes,scope.max_segment_bytes,
                scope.max_unfolded_tail_records
         FROM changed_path_scopes scope
         JOIN changed_path_observer_owners owner
           ON owner.scope_id=scope.scope_id AND owner.epoch=scope.epoch
         WHERE scope.scope_id=?1 AND scope.epoch=?2 AND scope.trust_state='trusted'
           AND owner.lease_state='active' AND owner.error_state IS NULL
           AND owner.expires_at>strftime('%s','now')",
        params![
            expected.scope_id.to_text(),
            i64::try_from(expected.epoch)
                .map_err(|_| Error::InvalidInput("lane epoch overflow".into()))?
        ],
        |row| {
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
            ))
        },
    )?;
    let owner_bytes: [u8; 32] = hex::decode(&owner_token)
        .map_err(|_| Error::Corrupt("invalid lane observer owner token".into()))?
        .try_into()
        .map_err(|_| Error::Corrupt("invalid lane observer owner token length".into()))?;
    let trail_directory = super::secure_fs::SecureDirectory::open_absolute(&db.db_dir)?;
    let observer_directory = trail_directory.open_dir("observer-segments")?;
    let secure_segment_directory = observer_directory.open_dir(&expected.scope_id.to_text())?;
    let recovery_scope = RecoveryScope {
        scope_id: expected.scope_id,
        epoch: expected.epoch,
        owner_token: owner_bytes,
    };
    let limits = PersistedLogLimits {
        max_log_bytes: u64::try_from(max_log_bytes)
            .map_err(|_| Error::Corrupt("negative observer log limit".into()))?,
        max_segment_bytes: u64::try_from(max_segment_bytes)
            .map_err(|_| Error::Corrupt("negative observer segment limit".into()))?,
        max_unfolded_tail_records: usize::try_from(max_tail_records)
            .map_err(|_| Error::Corrupt("invalid observer tail limit".into()))?,
    };
    let mut recovered = None;
    for attempt in 0..16 {
        let candidate = super::recover_segments_from_connection(
            &db.conn,
            &secure_segment_directory,
            &recovery_scope,
            limits,
        )
        .map_err(|error| Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: format!("controlled interval sidecar authentication failed: {error}"),
            command: command.clone(),
        })?;
        if !candidate.requires_reconciliation || attempt == 15 {
            recovered = Some(candidate);
            break;
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    let recovered = recovered.ok_or_else(|| {
        Error::Corrupt("controlled interval sidecar recovery did not produce a result".into())
    })?;
    if recovered.requires_reconciliation {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "controlled interval sidecar chain requires reconciliation".into(),
            command: command.clone(),
        });
    }
    let segment = recovered
        .segments
        .iter()
        .find(|segment| {
            segment.state == "sealed"
                && segment.last_sequence == c1.sequence
                && segment.durable_end_offset == c1.durable_offset
                && segment.folded_end_offset >= c1.folded_offset
                && Some(segment.start_cursor.as_slice()) == start_cursor.as_deref()
        })
        .ok_or_else(|| Error::ChangeLedgerReconcileRequired {
            scope: expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "prepared intent cut does not start the authenticated controlled interval"
                .into(),
            command,
        })?;
    let publication_boundary =
        authenticated_publication_boundary(&recovered, &c2, &publication_durable).ok_or_else(
            || Error::ChangeLedgerReconcileRequired {
                scope: expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: "publication cut is absent from the authenticated observer chain".into(),
                command: controlled_status_command(&runtime_key),
            },
        )?;
    Ok(QualifiedFilesystemProof {
        scope_id: expected.scope_id,
        epoch: expected.epoch,
        expected_root_id: crate::ObjectId(expected_root_id),
        scope_root_identity: hex::decode(scope_root_identity)
            .map_err(|_| Error::Corrupt("invalid lane scope root identity".into()))?,
        filesystem_identity: hex::decode(filesystem_identity)
            .map_err(|_| Error::Corrupt("invalid lane filesystem identity".into()))?,
        provider_id,
        provider_identity: hex::decode(provider_identity)
            .map_err(|_| Error::Corrupt("invalid lane provider identity".into()))?,
        observer_owner_token: owner_token,
        owner_fence_nonce: fence_nonce,
        durable_segment_id: segment.segment_id.clone(),
        durable_segment_hash: segment.segment_hash,
        segment_directory: format!("observer-segments/{}", expected.scope_id.to_text()),
        segment_path: segment.segment_path.clone(),
        start_cursor,
        end_cursor: segment.end_cursor.clone(),
        publication_segment_id: publication_boundary.0,
        publication_cursor: publication_boundary.1,
        start_sequence: segment.first_sequence,
        end_cut: c1.clone(),
        publication_cut: c2,
        segment_durable_offset: c1.durable_offset,
        segment_folded_offset: c1.folded_offset,
        verified_paths: evidence.exact_paths.len().try_into().unwrap_or(u64::MAX),
        verified_prefixes: evidence
            .complete_prefixes
            .len()
            .try_into()
            .unwrap_or(u64::MAX),
        complete_root_interval: true,
        complete_policy_interval: true,
        persisted_evidence_through_end: true,
    })
}

fn authenticated_publication_boundary(
    recovered: &RecoveredTail,
    publication_cut: &EvidenceCut,
    publication_durable: &DurableCut,
) -> Option<(String, Vec<u8>)> {
    recovered
        .record_boundaries
        .iter()
        .find(|boundary| {
            boundary.segment_id == publication_durable.segment_id
                && boundary.sequence == publication_cut.sequence
                && boundary.durable_end_offset == publication_cut.durable_offset
                && boundary.provider_cursor == publication_durable.provider_cursor
        })
        .map(|boundary| {
            (
                boundary.segment_id.clone(),
                boundary.provider_cursor.clone(),
            )
        })
        .or_else(|| {
            let first_sequence = publication_cut.sequence.checked_add(1)?;
            recovered
                .segments
                .iter()
                .find(|candidate| {
                    candidate.segment_id == publication_durable.segment_id
                        && matches!(candidate.state.as_str(), "open" | "sealed")
                        && candidate.start_cursor == publication_durable.provider_cursor
                        && candidate.first_sequence == first_sequence
                        && candidate.last_sequence >= publication_cut.sequence
                        && candidate.header_end_offset == publication_cut.durable_offset
                        && candidate.durable_end_offset >= publication_cut.durable_offset
                        && candidate.folded_end_offset >= publication_cut.folded_offset
                })
                .map(|candidate| (candidate.segment_id.clone(), candidate.start_cursor.clone()))
        })
}

pub(crate) fn materialized_lane_daemon_ready_proof(
    db: &Trail,
    lane: &str,
) -> Result<WorkspaceDaemonProof> {
    let lane_id = db.lane_branch(lane)?.lane_id;
    daemon_registry(db)
        .materialized_lanes
        .get(&lane_id)
        .ok_or_else(|| {
            Error::DaemonUnavailable(format!(
                "changed-path observer runtime for materialized lane `{lane_id}` is unavailable"
            ))
        })?
        .current_proof()
}

pub(crate) fn materialized_lane_daemon_marker_cut(
    db: &Trail,
    lane: &str,
) -> Result<Option<(EvidenceCut, String)>> {
    let lane_id = db.lane_branch(lane)?.lane_id;
    let registry = daemon_registry(db);
    let Some(runtime) = registry.materialized_lanes.get(&lane_id) else {
        return Ok(None);
    };
    let proof = runtime.current_proof()?;
    let anchor = runtime.tail_anchor.as_ref().ok_or_else(|| {
        Error::DaemonUnavailable("lane runtime has no authenticated marker anchor".into())
    })?;
    let durable = runtime
        .observer
        .authenticated_cut(&runtime.expected, anchor)?;
    if durable.last_sequence != proof.cut.sequence
        || durable.durable_end_offset != proof.cut.durable_offset
    {
        return Err(Error::ChangeLedgerReconcileRequired {
            scope: runtime.expected.scope_id.to_text(),
            state: "untrusted_gap".into(),
            reason: "lane marker runtime cut lost its exact sidecar boundary".into(),
            command: format!("trail lane status {lane_id}"),
        });
    }
    Ok(Some((proof.cut, durable.segment_id)))
}

pub(crate) fn materialized_lane_daemon_expected_scope(
    db: &Trail,
    lane: &str,
) -> Result<ExpectedScope> {
    let lane_id = db.lane_branch(lane)?.lane_id;
    if !daemon_registry(db)
        .materialized_lanes
        .contains_key(&lane_id)
    {
        prepare_materialized_lane_daemon(db, &lane_id, true)?;
    }
    daemon_registry(db)
        .materialized_lanes
        .get(&lane_id)
        .map(|runtime| runtime.expected.clone())
        .ok_or_else(|| Error::DaemonUnavailable("lane runtime disappeared".into()))
}

fn with_materialized_lane_runtime<T>(
    db: &Trail,
    lane: &str,
    operation: impl FnOnce(&mut WorkspaceDaemonRuntime, &Trail) -> Result<T>,
) -> Result<T> {
    let lane_id = db.lane_branch(lane)?.lane_id;
    let mut runtime = daemon_registry(db)
        .materialized_lanes
        .remove(&lane_id)
        .ok_or_else(|| {
            Error::DaemonUnavailable(format!(
                "changed-path observer runtime for materialized lane `{lane_id}` is unavailable"
            ))
        })?;
    let result = operation(&mut runtime, db);
    daemon_registry(db)
        .materialized_lanes
        .insert(lane_id, runtime);
    result
}

fn with_workspace_runtime<T>(
    db: &Trail,
    operation: impl FnOnce(&mut WorkspaceDaemonRuntime, &Trail) -> Result<T>,
) -> Result<T> {
    let mut runtime = daemon_registry(db).workspace.take().ok_or_else(|| {
        Error::DaemonUnavailable("changed-path workspace runtime is unavailable".into())
    })?;
    let result = operation(&mut runtime, db);
    daemon_registry(db).workspace = Some(runtime);
    result
}

fn controlled_runtime<'a>(
    registry: &'a ChangedPathDaemonRegistry,
    key: &ControlledRuntimeKey,
) -> Option<&'a WorkspaceDaemonRuntime> {
    match key {
        ControlledRuntimeKey::Workspace => registry.workspace.as_ref(),
        ControlledRuntimeKey::MaterializedLane(lane) => registry.materialized_lanes.get(lane),
    }
}

fn with_controlled_runtime<T>(
    db: &Trail,
    key: &ControlledRuntimeKey,
    operation: impl FnOnce(&mut WorkspaceDaemonRuntime, &Trail) -> Result<T>,
) -> Result<T> {
    match key {
        ControlledRuntimeKey::Workspace => with_workspace_runtime(db, operation),
        ControlledRuntimeKey::MaterializedLane(lane) => {
            with_materialized_lane_runtime(db, lane, operation)
        }
    }
}

fn ensure_controlled_runtime(db: &Trail, key: &ControlledRuntimeKey) -> Result<()> {
    match key {
        ControlledRuntimeKey::Workspace => {
            if daemon_registry(db).workspace.is_none() {
                prepare_workspace_daemon(db, true)?;
            }
        }
        ControlledRuntimeKey::MaterializedLane(lane) => {
            if !daemon_registry(db).materialized_lanes.contains_key(lane) {
                prepare_materialized_lane_daemon(db, lane, true)?;
            }
        }
    }
    Ok(())
}

fn controlled_runtime_matches_target(db: &Trail, key: &ControlledRuntimeKey) -> Result<bool> {
    let registry = daemon_registry(db);
    let Some(runtime) = controlled_runtime(&registry, key) else {
        return Ok(false);
    };
    let target = match key {
        ControlledRuntimeKey::Workspace => workspace_daemon_target(db)?,
        ControlledRuntimeKey::MaterializedLane(lane) => materialized_lane_daemon_target(db, lane)?,
    };
    runtime.matches_target(&target)
}

fn controlled_status_command(key: &ControlledRuntimeKey) -> String {
    match key {
        ControlledRuntimeKey::Workspace => "trail status".into(),
        ControlledRuntimeKey::MaterializedLane(lane) => format!("trail lane status {lane}"),
    }
}

fn daemon_registry(db: &Trail) -> std::sync::MutexGuard<'_, ChangedPathDaemonRegistry> {
    db.changed_path_daemon_registry
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
}

pub(crate) struct WorkspaceDaemonRuntime {
    root: std::path::PathBuf,
    scope_kind: ScopeKind,
    expected: ExpectedScope,
    policy: CompiledPolicy,
    observer: PlatformObserver,
    tail_anchor: Option<ObserverFence>,
    last_cut: Option<EvidenceCut>,
    daemon_launch_nonce: Option<String>,
    #[cfg(all(test, target_os = "macos"))]
    inject_policy_drift_after_end: bool,
}

impl WorkspaceDaemonRuntime {
    fn start(
        db: &Trail,
        target: DaemonScopeTarget,
        replace_stale_owner: bool,
        refuse_live_unverified_owner: bool,
        verified_stale_owner: Option<&VerifiedStaleWorkspaceOwner>,
        launch: Option<&WorkspaceDaemonLaunchIdentity>,
    ) -> Result<Self> {
        let DaemonScopeTarget {
            root: scope_root,
            identity: scope_identity,
            baseline,
        } = target;
        let scope_id = scope_identity.scope_id;
        let scope_kind = scope_identity.kind;
        let segment_directory = db.db_dir.join("observer-segments").join(scope_id.to_text());
        fs::create_dir_all(&segment_directory)?;
        let filesystem_identity = root_identity(&scope_root)?;
        let provider_identity = platform_provider_identity();
        let capabilities = platform_capabilities();
        let ledger = db.changed_path_ledger();
        let existing = load_existing_scope(db, scope_id).map_err(|error| {
            Error::DaemonUnavailable(format!(
                "changed-path observer could not load its persisted scope: {error}"
            ))
        })?;
        let case_sensitive = match existing
            .as_ref()
            .filter(|stored| stored.filesystem_identity == filesystem_identity)
            .map(|stored| stored.case_sensitive)
        {
            Some(0) => false,
            Some(1) => true,
            Some(_) => {
                return Err(Error::Corrupt(
                    "changed-path scope has invalid case-sensitivity metadata".into(),
                ))
            }
            None => !crate::db::util::is_case_insensitive_filesystem(&scope_root)?,
        };
        #[cfg(debug_assertions)]
        if existing.is_some() {
            test_daemon_transition_after_load_boundary()?;
        }
        let (epoch, policy_generation, mut resume_cursor) = match existing {
            None => {
                ledger
                    .begin_scope(
                        &scope_identity,
                        &baseline,
                        &PolicyIdentity {
                            fingerprint: [0; 32],
                            generation: 1,
                        },
                        &FilesystemIdentity(filesystem_identity.clone()),
                        &ProviderIdentity {
                            identity: provider_identity.clone(),
                            capabilities: capabilities.clone(),
                        },
                    )
                    .map_err(|error| {
                        Error::DaemonUnavailable(format!(
                            "changed-path observer could not initialize its scope: {error}"
                        ))
                    })?;
                (1, 1, None)
            }
            Some(stored) => {
                let policy_generation = stored.policy_generation;
                let current_filesystem_identity = hex::encode(&filesystem_identity);
                let current_provider_identity = hex::encode(&provider_identity);
                let identity_changed = stored.filesystem_identity != filesystem_identity
                    || stored.scope_root_identity != current_filesystem_identity
                    || stored.provider_identity != provider_identity
                    || stored.provider_id_text.as_deref()
                        != Some(current_provider_identity.as_str());
                if identity_changed && !replace_stale_owner {
                    return Err(Error::ChangeLedgerReconcileRequired {
                        scope: scope_id.to_text(),
                        state: "stale_baseline".into(),
                        reason: "persisted daemon scope does not match the exact current baseline"
                            .into(),
                        command: "trail status".into(),
                    });
                }
                let baseline_changed = stored.ref_name != baseline.ref_name
                    || stored.ref_generation != baseline.ref_generation
                    || stored.change_id != baseline.change_id.0
                    || stored.baseline_root != baseline.root_id.0;
                let old_expected = ExpectedScope {
                    scope_id,
                    epoch: stored.epoch,
                    ref_name: stored.ref_name.clone(),
                    ref_generation: stored.ref_generation,
                    baseline_root: crate::ObjectId(stored.baseline_root.clone()),
                    policy_fingerprint: stored.policy_fingerprint,
                    policy_generation: stored.policy_generation,
                    filesystem_identity: stored.filesystem_identity.clone(),
                    provider_identity: stored.provider_identity.clone(),
                };
                if !identity_changed {
                    ledger.recover_scope(&old_expected)?;
                }
                if !replace_stale_owner {
                    return Err(Error::DaemonUnavailable(
                        "persisted workspace daemon owner exists without verified stale process identity"
                            .into(),
                    ));
                }
                if let Some(verified) = verified_stale_owner {
                    let mut invalid = Vec::new();
                    if verified.scope_id != scope_id.to_text() {
                        invalid.push("scope");
                    }
                    if verified.epoch != stored.epoch {
                        invalid.push("scope_epoch");
                    }
                    match stored.observer_owner.as_ref() {
                        Some(owner) => {
                            if owner.daemon_launch_nonce.as_deref()
                                != Some(verified.daemon_launch_nonce.as_str())
                            {
                                invalid.push("launch_nonce");
                            }
                            if owner.daemon_pid != Some(i64::from(verified.stale_pid)) {
                                invalid.push("pid");
                            }
                            if owner.daemon_process_start_identity.as_deref()
                                != Some(verified.process_start_identity.as_str())
                            {
                                invalid.push("process_start_identity");
                            }
                            if owner.epoch != verified.epoch {
                                invalid.push("owner_epoch");
                            }
                            if owner.owner_token != verified.observer_owner_token {
                                invalid.push("owner_token");
                            }
                        }
                        None => invalid.push("owner"),
                    }
                    if stored.observer_owner_token.as_deref()
                        != Some(verified.observer_owner_token.as_str())
                    {
                        invalid.push("scope_owner_token");
                    }
                    if !invalid.is_empty() {
                        return Err(Error::DaemonUnavailable(format!(
                            "verified stale daemon PID {} ({}) does not match the exact persisted observer scope/epoch/owner token ({})",
                            verified.stale_pid, verified.process_start_identity, invalid.join(", ")
                        )));
                    }
                } else if refuse_live_unverified_owner
                    && stored.observer_owner.as_ref().is_some_and(|owner| {
                        owner.lease_state == "active"
                            && owner.error_state.is_none()
                            && owner.expires_at > crate::db::util::now_ts()
                    })
                {
                    return Err(Error::DaemonUnavailable(
                        "changed-path observer owner is still live; refusing unverified authority replacement"
                            .into(),
                    ));
                }
                let old_epoch = stored.epoch;
                let next = old_epoch.checked_add(1).ok_or_else(|| {
                    Error::InvalidInput("changed-path daemon scope epoch overflow".into())
                })?;
                let owner_authority_consistent = match stored.observer_owner.as_ref() {
                    Some(owner) => {
                        let provider_binding_matches_loaded_scope =
                            stored.provider_id_text.as_deref() == Some(owner.provider_id.as_str())
                                && stored.provider_identity_text.as_deref()
                                    == Some(owner.provider_identity.as_str());
                        let provider_binding_is_verified_drift_target = identity_changed
                            && owner.provider_id == current_provider_identity
                            && owner.provider_identity == current_provider_identity;
                        owner.epoch == stored.epoch
                            && stored.observer_owner_token.as_deref()
                                == Some(owner.owner_token.as_str())
                            && (provider_binding_matches_loaded_scope
                                || provider_binding_is_verified_drift_target)
                    }
                    None => stored.observer_owner_token.is_none() || !refuse_live_unverified_owner,
                };
                if !owner_authority_consistent {
                    return Err(daemon_owner_authority_inconsistent(scope_id));
                }
                let tx = db.conn.unchecked_transaction()?;
                let owner_changed = match stored.observer_owner.as_ref() {
                    Some(owner) => tx.execute(
                        "UPDATE changed_path_observer_owners
                         SET lease_state='revoked', error_state='daemon_owner_replaced',
                             error_at=strftime('%s','now'), updated_at=strftime('%s','now')
                         WHERE scope_id=:scope_id AND epoch=:epoch
                           AND owner_token=:owner_token AND provider_id=:provider_id
                           AND provider_identity=:provider_identity
                           AND lease_state=:lease_state AND fence_nonce IS :fence_nonce
                           AND acquired_at=:acquired_at AND heartbeat_at=:heartbeat_at
                           AND expires_at=:expires_at AND error_state IS :error_state
                           AND error_at IS :error_at",
                        named_params! {
                            ":scope_id": scope_id.to_text(),
                            ":epoch": i64::try_from(owner.epoch).map_err(|_| Error::InvalidInput("observer epoch overflow".into()))?,
                            ":owner_token": &owner.owner_token,
                            ":provider_id": &owner.provider_id,
                            ":provider_identity": &owner.provider_identity,
                            ":lease_state": &owner.lease_state,
                            ":fence_nonce": &owner.fence_nonce,
                            ":acquired_at": owner.acquired_at,
                            ":heartbeat_at": owner.heartbeat_at,
                            ":expires_at": owner.expires_at,
                            ":error_state": &owner.error_state,
                            ":error_at": owner.error_at,
                        },
                    )?,
                    None => {
                        let count = tx.query_row(
                            "SELECT COUNT(*) FROM changed_path_observer_owners WHERE scope_id=?1",
                            [scope_id.to_text()],
                            |row| row.get::<_, i64>(0),
                        )?;
                        usize::from(count == 0)
                    }
                };
                if owner_changed != 1 {
                    tx.rollback()?;
                    return Err(daemon_authority_transition_lost(scope_id));
                }
                let owner_revocation_triggered = stored
                    .observer_owner
                    .as_ref()
                    .is_some_and(|owner| owner.lease_state == "active");
                let old_trust_state_after_revocation = if owner_revocation_triggered
                    && matches!(stored.trust_state.as_str(), "trusted" | "reconciling")
                {
                    "untrusted_gap"
                } else {
                    stored.trust_state.as_str()
                };
                let old_trust_reason_after_revocation = if owner_revocation_triggered
                    && matches!(stored.trust_state.as_str(), "trusted" | "reconciling")
                {
                    "observer_owner_revoked"
                } else {
                    stored.trust_reason.as_str()
                };
                let old_continuity_after_revocation = stored
                    .continuity_generation
                    .checked_add(u64::from(owner_revocation_triggered))
                    .ok_or_else(|| Error::InvalidInput("continuity generation overflow".into()))?;
                let changed = tx.execute(
                    "UPDATE changed_path_scopes
                     SET epoch=:next_epoch,
                         ref_name=:next_ref_name, ref_generation=:next_ref_generation,
                         change_id=:next_change_id, baseline_root_id=:next_baseline_root,
                         scope_root_identity=:next_filesystem_identity,
                         filesystem_identity=:next_filesystem_identity,
                         provider_id=:next_provider_identity,
                         provider_identity=:next_provider_identity,
                         durable_cursor=:next_durable_cursor,
                         linearizable_fence=:next_linearizable_fence,
                         rename_pairing=:next_rename_pairing,
                         overflow_scope=:next_overflow_scope,
                         filesystem_supported=:next_filesystem_supported,
                         clean_proof_allowed=:next_clean_proof_allowed,
                         power_loss_durability=:next_power_loss_durability,
                         trust_state='untrusted_gap', trust_reason=:next_trust_reason,
                         observer_owner_token=NULL, provider_cursor=NULL, provider_fence=NULL,
                         observer_heartbeat_at=NULL, observer_error_state=NULL,
                         observer_error_at=NULL,
                         durable_offset=0, folded_offset=0,
                         continuity_generation=continuity_generation+1,
                         updated_at=strftime('%s','now')
                     WHERE scope_id=:scope_id AND scope_kind=:old_scope_kind
                       AND schema_version=:old_schema_version
                       AND owner_id=:old_owner_id AND scope_root=:old_scope_root
                       AND scope_root_identity=:old_scope_root_identity
                       AND filesystem_identity=:old_filesystem_identity
                       AND filesystem_kind=:old_filesystem_kind
                       AND case_sensitive=:old_case_sensitive
                       AND epoch=:old_epoch AND ref_name=:old_ref_name
                       AND ref_generation=:old_ref_generation
                       AND change_id=:old_change_id
                       AND baseline_root_id=:old_baseline_root
                       AND policy_fingerprint=:old_policy_fingerprint
                       AND policy_dependency_generation=:old_policy_generation
                       AND trust_state=:old_trust_state
                       AND trust_reason=:old_trust_reason
                       AND continuity_generation=:old_continuity_generation
                       AND provider_id IS :old_provider_id
                       AND provider_identity IS :old_provider_identity
                       AND durable_cursor=:old_durable_cursor
                       AND linearizable_fence=:old_linearizable_fence
                       AND rename_pairing=:old_rename_pairing
                       AND overflow_scope=:old_overflow_scope
                       AND filesystem_supported=:old_filesystem_supported
                       AND clean_proof_allowed=:old_clean_proof_allowed
                       AND power_loss_durability=:old_power_loss_durability
                       AND provider_cursor IS :old_provider_cursor
                       AND provider_fence IS :old_provider_fence
                       AND durable_offset=:old_durable_offset
                       AND folded_offset=:old_folded_offset
                       AND observer_owner_token IS :old_observer_owner_token
                       AND observer_heartbeat_at IS :old_observer_heartbeat_at
                       AND observer_error_state IS :old_observer_error_state
                       AND observer_error_at IS :old_observer_error_at
                       AND max_candidate_rows=:old_max_candidate_rows
                       AND max_prefix_rows=:old_max_prefix_rows
                       AND max_observer_log_bytes=:old_max_observer_log_bytes
                       AND max_segment_bytes=:old_max_segment_bytes
                       AND max_unfolded_tail_records=:old_max_unfolded_tail_records
                       AND retired_at IS :old_retired_at",
                    named_params! {
                        ":next_epoch": i64::try_from(next).map_err(|_| Error::InvalidInput("epoch overflow".into()))?,
                        ":next_ref_name": &baseline.ref_name,
                        ":next_ref_generation": i64::try_from(baseline.ref_generation).map_err(|_| Error::InvalidInput("ref generation overflow".into()))?,
                        ":next_change_id": &baseline.change_id.0,
                        ":next_baseline_root": &baseline.root_id.0,
                        ":next_filesystem_identity": hex::encode(&filesystem_identity),
                        ":next_provider_identity": hex::encode(&provider_identity),
                        ":next_durable_cursor": i64::from(capabilities.durable_cursor),
                        ":next_linearizable_fence": i64::from(capabilities.linearizable_fence),
                        ":next_rename_pairing": i64::from(capabilities.rename_pairing),
                        ":next_overflow_scope": i64::from(capabilities.overflow_scope),
                        ":next_filesystem_supported": i64::from(capabilities.filesystem_supported),
                        ":next_clean_proof_allowed": i64::from(capabilities.clean_proof_allowed),
                        ":next_power_loss_durability": i64::from(capabilities.power_loss_durability),
                        ":next_trust_reason": if identity_changed { "daemon_identity_transition" } else { "daemon_owner_restarted" },
                        ":scope_id": scope_id.to_text(),
                        ":old_scope_kind": &stored.scope_kind,
                        ":old_schema_version": stored.schema_version,
                        ":old_owner_id": &stored.owner_id,
                        ":old_scope_root": &stored.scope_root,
                        ":old_scope_root_identity": &stored.scope_root_identity,
                        ":old_filesystem_identity": hex::encode(&stored.filesystem_identity),
                        ":old_filesystem_kind": &stored.filesystem_kind,
                        ":old_case_sensitive": stored.case_sensitive,
                        ":old_epoch": i64::try_from(old_epoch).map_err(|_| Error::InvalidInput("epoch overflow".into()))?,
                        ":old_ref_name": &stored.ref_name,
                        ":old_ref_generation": i64::try_from(stored.ref_generation).map_err(|_| Error::InvalidInput("ref generation overflow".into()))?,
                        ":old_change_id": &stored.change_id,
                        ":old_baseline_root": &stored.baseline_root,
                        ":old_policy_fingerprint": hex::encode(stored.policy_fingerprint),
                        ":old_policy_generation": i64::try_from(stored.policy_generation).map_err(|_| Error::InvalidInput("policy generation overflow".into()))?,
                        ":old_trust_state": old_trust_state_after_revocation,
                        ":old_trust_reason": old_trust_reason_after_revocation,
                        ":old_continuity_generation": i64::try_from(old_continuity_after_revocation).map_err(|_| Error::InvalidInput("continuity generation overflow".into()))?,
                        ":old_provider_id": &stored.provider_id_text,
                        ":old_provider_identity": &stored.provider_identity_text,
                        ":old_durable_cursor": stored.capabilities[0],
                        ":old_linearizable_fence": stored.capabilities[1],
                        ":old_rename_pairing": stored.capabilities[2],
                        ":old_overflow_scope": stored.capabilities[3],
                        ":old_filesystem_supported": stored.capabilities[4],
                        ":old_clean_proof_allowed": stored.capabilities[5],
                        ":old_power_loss_durability": stored.capabilities[6],
                        ":old_provider_cursor": &stored.provider_cursor,
                        ":old_provider_fence": &stored.provider_fence,
                        ":old_durable_offset": i64::try_from(stored.durable_offset).map_err(|_| Error::InvalidInput("durable offset overflow".into()))?,
                        ":old_folded_offset": i64::try_from(stored.folded_offset).map_err(|_| Error::InvalidInput("folded offset overflow".into()))?,
                        ":old_observer_owner_token": &stored.observer_owner_token,
                        ":old_observer_heartbeat_at": stored.observer_heartbeat_at,
                        ":old_observer_error_state": &stored.observer_error_state,
                        ":old_observer_error_at": stored.observer_error_at,
                        ":old_retired_at": stored.retired_at,
                        ":old_max_candidate_rows": stored.limits[0],
                        ":old_max_prefix_rows": stored.limits[1],
                        ":old_max_observer_log_bytes": stored.limits[2],
                        ":old_max_segment_bytes": stored.limits[3],
                        ":old_max_unfolded_tail_records": stored.limits[4],
                    },
                )?;
                if changed != 1 {
                    tx.rollback()?;
                    return Err(daemon_authority_transition_lost(scope_id));
                }
                tx.commit()?;
                (
                    next,
                    policy_generation,
                    if baseline_changed || identity_changed {
                        None
                    } else {
                        stored.provider_cursor
                    },
                )
            }
        };

        let mut expected = ExpectedScope {
            scope_id,
            epoch,
            ref_name: baseline.ref_name,
            ref_generation: baseline.ref_generation,
            baseline_root: baseline.root_id,
            policy_fingerprint: [0; 32],
            policy_generation,
            filesystem_identity,
            provider_identity,
        };
        let stored_fingerprint: String = db
            .conn
            .query_row(
                "SELECT policy_fingerprint FROM changed_path_scopes WHERE scope_id=?1 AND epoch=?2",
                params![scope_id.to_text(), i64::try_from(epoch).unwrap_or(i64::MAX)],
                |row| row.get(0),
            )
            .map_err(|error| {
                Error::DaemonUnavailable(format!(
                    "changed-path observer could not read its policy identity: {error}"
                ))
            })?;
        expected.policy_fingerprint = decode_fingerprint(&stored_fingerprint)?;
        let git_environment = std::env::vars_os().collect::<Vec<(OsString, OsString)>>();
        let mut metrics = PolicyDependencyMetrics::default();
        let mut policy = compile_policy(
            &db.conn,
            &expected,
            &PolicyCompileContext {
                workspace_root: &scope_root,
                db_dir: &db.db_dir,
                recording: &db.config.recording,
                case_sensitive,
                git_environment: &git_environment,
            },
            &mut metrics,
        )
        .map_err(|error| {
            Error::DaemonUnavailable(format!(
                "changed-path observer could not compile its recording policy: {error}"
            ))
        })?;
        if expected.policy_fingerprint == [0; 32] {
            expected.policy_fingerprint = policy.fingerprint();
            db.conn.execute(
                "UPDATE changed_path_scopes SET policy_fingerprint=?1, updated_at=strftime('%s','now')
                 WHERE scope_id=?2 AND epoch=?3 AND policy_fingerprint=?4",
                params![
                    hex::encode(expected.policy_fingerprint),
                    scope_id.to_text(),
                    i64::try_from(epoch).unwrap_or(i64::MAX),
                    stored_fingerprint,
                ],
            )?;
        } else if expected.policy_fingerprint != policy.fingerprint() {
            let previous_fingerprint = expected.policy_fingerprint;
            let previous_generation = expected.policy_generation;
            let next_generation = previous_generation.checked_add(1).ok_or_else(|| {
                Error::InvalidInput("changed-path policy generation overflow".into())
            })?;
            let next_fingerprint = policy.fingerprint();
            let tx = db.conn.unchecked_transaction()?;
            tx.execute(
                "UPDATE changed_path_policy_dependencies SET generation=?1, updated_at=strftime('%s','now')
                 WHERE scope_id=?2 AND generation=?3",
                params![
                    i64::try_from(next_generation)
                        .map_err(|_| Error::InvalidInput("policy generation overflow".into()))?,
                    scope_id.to_text(),
                    i64::try_from(previous_generation)
                        .map_err(|_| Error::InvalidInput("policy generation overflow".into()))?,
                ],
            )?;
            let changed = tx.execute(
                "UPDATE changed_path_scopes
                 SET policy_fingerprint=?1, policy_dependency_generation=?2,
                     trust_state='untrusted_gap', trust_reason='daemon_policy_transition',
                     provider_cursor=NULL, provider_fence=NULL,
                     continuity_generation=continuity_generation+1,
                     updated_at=strftime('%s','now')
                 WHERE scope_id=?3 AND epoch=?4 AND policy_fingerprint=?5
                   AND policy_dependency_generation=?6",
                params![
                    hex::encode(next_fingerprint),
                    i64::try_from(next_generation)
                        .map_err(|_| Error::InvalidInput("policy generation overflow".into()))?,
                    scope_id.to_text(),
                    i64::try_from(epoch)
                        .map_err(|_| Error::InvalidInput("epoch overflow".into()))?,
                    hex::encode(previous_fingerprint),
                    i64::try_from(previous_generation)
                        .map_err(|_| Error::InvalidInput("policy generation overflow".into()))?,
                ],
            )?;
            if changed != 1 {
                return Err(Error::ChangeLedgerReconcileRequired {
                    scope: scope_id.to_text(),
                    state: "stale_baseline".into(),
                    reason: "daemon policy transition lost exact scope authority".into(),
                    command: "trail status".into(),
                });
            }
            tx.commit()?;
            expected.policy_fingerprint = next_fingerprint;
            expected.policy_generation = next_generation;
            resume_cursor = None;
        }

        let mut owner = [0_u8; 32];
        let mut fence_nonce = [0_u8; 24];
        getrandom(&mut owner)
            .map_err(|error| Error::InvalidInput(format!("observer owner entropy: {error}")))?;
        getrandom(&mut fence_nonce)
            .map_err(|error| Error::InvalidInput(format!("observer fence entropy: {error}")))?;
        let primary_autocommit = db.conn.is_autocommit();
        let writer = match launch {
            Some(launch) => SegmentWriter::acquire_for_daemon(
                &db.sqlite_path,
                &segment_directory,
                scope_id,
                epoch,
                owner,
                &hex::encode(&expected.provider_identity),
                resume_cursor.clone().unwrap_or_default(),
                Duration::from_secs(30),
                DaemonLaunchBinding {
                    nonce: launch.nonce.clone(),
                    pid: launch.pid,
                    process_start_identity: launch.process_start_identity.clone(),
                },
            ),
            None => SegmentWriter::acquire(
                &db.sqlite_path,
                &segment_directory,
                scope_id,
                epoch,
                owner,
                &hex::encode(&expected.provider_identity),
                resume_cursor.clone().unwrap_or_default(),
                Duration::from_secs(30),
            ),
        }
        .map_err(|error| {
            Error::DaemonUnavailable(format!(
                "changed-path observer could not acquire its durable segment writer (primary_autocommit={primary_autocommit}): {error}"
            ))
        })?;
        let observer = PlatformObserver::start(
            &scope_root,
            writer,
            expected.provider_identity.clone(),
            fence_nonce.to_vec(),
            policy.dependency_files(),
            resume_cursor.take(),
        )
        .map_err(|error| {
            Error::DaemonUnavailable(format!(
                "changed-path observer could not start native coverage: {error}"
            ))
        })?;
        let compile_start = observer.begin_observation(&expected).map_err(|error| {
            Error::DaemonUnavailable(format!(
                "changed-path observer could not establish its policy-compile start fence: {error}"
            ))
        })?;
        let mut verified_metrics = PolicyDependencyMetrics::default();
        let verified_policy = revalidate_compiled_policy(
            &policy,
            &PolicyCompileContext {
                workspace_root: &scope_root,
                db_dir: &db.db_dir,
                recording: &db.config.recording,
                case_sensitive,
                git_environment: &git_environment,
            },
            &mut verified_metrics,
        )
        .map_err(|error| {
            Error::DaemonUnavailable(format!(
                "changed-path observer could not verify its recording policy: {error}"
            ))
        })?;
        db.note_operation_metrics(crate::db::OperationMetricsDelta {
            policy_dependency_full_discovery: metrics
                .policy_dependency_full_discovery
                .saturating_add(verified_metrics.policy_dependency_full_discovery),
            ..crate::db::OperationMetricsDelta::default()
        });
        let compile_end = observer
            .end_fence(&expected, &compile_start)
            .map_err(|error| {
                Error::DaemonUnavailable(format!(
                    "changed-path observer could not establish its policy-compile end fence: {error}"
                ))
            })?;
        let lease = observer.lease().map_err(|error| {
            Error::DaemonUnavailable(format!(
                "changed-path observer could not authenticate its startup lease: {error}"
            ))
        })?;
        observer
            .drain_through(
                &expected,
                &lease.root_identity,
                &compile_start,
                &compile_end,
                &mut |_event| Ok(()),
            )
            .map_err(|error| {
                Error::DaemonUnavailable(format!(
                    "changed-path observer could not drain its policy-compile interval: {error}"
                ))
            })?;
        if verified_policy.fingerprint() != policy.fingerprint()
            || verified_policy.dependency_files() != policy.dependency_files()
        {
            return Err(Error::DaemonUnavailable(
                "recording policy changed during observer startup; retrying requires a fresh full reconciliation"
                    .into(),
            ));
        }
        policy = verified_policy;
        policy.authorize_native_reconciliation(&expected, &lease)?;
        Ok(Self {
            root: scope_root,
            scope_kind,
            expected,
            policy,
            observer,
            tail_anchor: None,
            last_cut: None,
            daemon_launch_nonce: launch.map(|launch| launch.nonce.clone()),
            #[cfg(all(test, target_os = "macos"))]
            inject_policy_drift_after_end: false,
        })
    }

    fn matches_target(&self, target: &DaemonScopeTarget) -> Result<bool> {
        Ok(self.root == target.root
            && self.expected.scope_id == target.identity.scope_id
            && self.expected.ref_name == target.baseline.ref_name
            && self.expected.ref_generation == target.baseline.ref_generation
            && self.expected.baseline_root == target.baseline.root_id
            && self.expected.filesystem_identity == root_identity(&target.root)?)
    }

    fn validate_request(&self, scope_id: Option<&str>, epoch: Option<u64>) -> Result<()> {
        if scope_id.is_some_and(|scope| scope != self.expected.scope_id.to_text())
            || epoch.is_some_and(|epoch| epoch != self.expected.epoch)
        {
            return Err(Error::DaemonUnavailable(
                "changed-path daemon RPC scope or epoch mismatch".into(),
            ));
        }
        Ok(())
    }

    fn reconcile(&mut self, db: &Trail, reason: &str) -> Result<WorkspaceDaemonProof> {
        db.note_operation_metrics(crate::db::OperationMetricsDelta {
            reconciliation_run_count: 1,
            ..crate::db::OperationMetricsDelta::default()
        });
        let ledger = db.changed_path_ledger();
        let previous_state = ledger
            .conn
            .query_row(
                "SELECT trust_state FROM changed_path_scopes WHERE scope_id=?1",
                [self.expected.scope_id.to_text()],
                |row| row.get::<_, String>(0),
            )
            .unwrap_or_else(|_| "uninitialized".to_string());
        ledger
            .recover_scope(&self.expected)
            .map_err(|error| match error {
                Error::Sqlite(_) => Error::DaemonUnavailable(format!(
                "changed-path reconciliation could not recover its durable sidecar state: {error}"
            )),
                other => other,
            })?;
        let (mut report, tail_anchor) = reconcile_full_with_tail(
            db,
            &ledger,
            &self.observer,
            &self.expected,
            &self.policy,
            reason,
        )
        .map_err(|error| match error {
            Error::Sqlite(_) => Error::DaemonUnavailable(format!(
                "changed-path reconciliation could not complete its scan and observer-tail fold: {error}"
            )),
            other => other,
        })?;
        let cut = EvidenceCut {
            source: EvidenceSource::Observer,
            sequence: tail_anchor.sequence,
            durable_offset: tail_anchor.durable_offset,
            folded_offset: tail_anchor.durable_offset,
        };
        self.tail_anchor = Some(tail_anchor);
        self.last_cut = Some(cut.clone());
        report.scope_id = self.expected.scope_id.to_text();
        report.scope_kind = self.scope_kind.as_str().to_string();
        report.previous_state = previous_state;
        report.observed_paths = report.observed_files;
        report.candidates = report.candidate_rows;
        report.resulting_epoch = self.expected.epoch;
        report.resulting_state = report.trust_state.clone();
        let observer_owner_token = self.observer.lease()?.owner_token;
        Ok(WorkspaceDaemonProof {
            scope_id: self.expected.scope_id.to_text(),
            epoch: self.expected.epoch,
            observer_owner_token,
            daemon_launch_nonce: self.daemon_launch_nonce.clone(),
            cut,
            reconcile_report: Some(report),
        })
    }

    fn fence(&mut self, db: &Trail) -> Result<WorkspaceDaemonProof> {
        self.fence_with_activity(db).map(|(proof, _)| proof)
    }

    fn fence_with_activity(&mut self, db: &Trail) -> Result<(WorkspaceDaemonProof, usize)> {
        let start = self.tail_anchor.clone().ok_or_else(|| {
            Error::DaemonUnavailable("workspace daemon has no continuous observer anchor".into())
        })?;
        let fence = self.observer.end_fence(&self.expected, &start)?;
        self.fold_end_fence_with_activity(db, start, fence)
    }

    fn fold_end_fence_with_activity(
        &mut self,
        db: &Trail,
        start: ObserverFence,
        fence: ObserverFence,
    ) -> Result<(WorkspaceDaemonProof, usize)> {
        #[cfg(all(test, target_os = "macos"))]
        if self.inject_policy_drift_after_end {
            self.inject_policy_drift_after_end = false;
            self.observer.fail_next_direct_policy_fence_for_test();
        }
        // Preserve typed policy invalidation so the command wrapper can
        // revoke/restart/recompile and automatically full-reconcile. Wrapping
        // it as generic daemon unavailability would strand a revoked runtime.
        let lease = self.observer.lease()?;
        let mut events = Vec::<ObserverEvent>::new();
        let mut qualification = self.observer.drain_through_retaining_end(
            &self.expected,
            &lease.root_identity,
            &start,
            &fence,
            &mut |event| {
                if raw_event_invalidates_policy(
                    &self.policy,
                    std::path::Path::new(event.path.as_str()),
                ) {
                    return Err(Error::ChangeLedgerReconcileRequired {
                        scope: self.expected.scope_id.to_text(),
                        state: "stale_baseline".into(),
                        reason: "recording policy changed during continuous observer interval"
                            .into(),
                        command: "trail status".into(),
                    });
                }
                events.push(event);
                Ok(())
            },
        )?;
        let cut = fold_observer_interval(
            &db.changed_path_ledger(),
            &self.expected,
            &lease.root_identity,
            &start,
            &fence,
            &mut qualification,
            &events,
        )?;
        let authenticated_end = self.observer.authenticated_cut(&self.expected, &fence)?;
        let cursor_changed = db.conn.execute(
            "UPDATE changed_path_scopes SET provider_cursor=?1,updated_at=?2
             WHERE scope_id=?3 AND epoch=?4 AND durable_offset=?5 AND folded_offset=?5
               AND observer_owner_token=?6 AND trust_state='trusted'",
            params![
                authenticated_end.provider_cursor,
                crate::db::util::now_ts(),
                self.expected.scope_id.to_text(),
                i64::try_from(self.expected.epoch)
                    .map_err(|_| Error::InvalidInput("observer epoch overflow".into()))?,
                i64::try_from(fence.durable_offset)
                    .map_err(|_| Error::InvalidInput("observer fence offset overflow".into()))?,
                lease.owner_token,
            ],
        )?;
        if cursor_changed != 1 {
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: self.expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: "continuous observer cursor lost its exact scope CAS".into(),
                command: "trail status".into(),
            });
        }
        let observed_activity = events
            .iter()
            .filter(|event| !event.path.as_str().starts_with(".trail/observer-fences/"))
            .count();
        db.note_operation_metrics(crate::db::OperationMetricsDelta {
            observer_tail_record_fold_count: events.len().try_into().unwrap_or(u64::MAX),
            ledger_row_touch_count: events.len().try_into().unwrap_or(u64::MAX),
            ..crate::db::OperationMetricsDelta::default()
        });
        self.tail_anchor = Some(fence);
        self.last_cut = Some(cut.clone());
        Ok((
            WorkspaceDaemonProof {
                scope_id: self.expected.scope_id.to_text(),
                epoch: self.expected.epoch,
                observer_owner_token: lease.owner_token,
                daemon_launch_nonce: self.daemon_launch_nonce.clone(),
                cut,
                reconcile_report: None,
            },
            observed_activity,
        ))
    }

    /// Fence, fold, and seal an exact observer segment boundary. The native
    /// observer appends its controlled fence and rotates in one durability
    /// worker turn, so traffic ordered after the fence starts in the linked
    /// segment instead of starving the boundary.
    fn fence_and_seal(&mut self, db: &Trail) -> Result<WorkspaceDaemonProof> {
        let start = self.tail_anchor.clone().ok_or_else(|| {
            Error::DaemonUnavailable("workspace daemon has no continuous observer anchor".into())
        })?;
        let (end, sealed, anchor_cut) =
            self.observer.controlled_end_fence(&self.expected, &start)?;
        let (proof, _) = self.fold_end_fence_with_activity(db, start, end.clone())?;
        if sealed.last_sequence != proof.cut.sequence
            || sealed.durable_end_offset != proof.cut.durable_offset
            || sealed.last_sequence != anchor_cut.last_sequence
            || sealed.provider_cursor != anchor_cut.provider_cursor
        {
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: self.expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: "controlled observer fence did not equal its atomically sealed cut".into(),
                command: "trail lane status".into(),
            });
        }
        let anchor =
            self.observer
                .install_rotation_anchor(&self.expected, &end, anchor_cut.clone())?;
        self.advance_rotation_anchor(db, &sealed, &anchor_cut)?;
        self.tail_anchor = Some(anchor);
        self.last_cut = Some(EvidenceCut {
            source: EvidenceSource::Observer,
            sequence: anchor_cut.last_sequence,
            durable_offset: anchor_cut.durable_end_offset,
            folded_offset: anchor_cut.durable_end_offset,
        });
        Ok(proof)
    }

    fn advance_rotation_anchor(
        &self,
        db: &Trail,
        sealed: &DurableCut,
        anchor: &DurableCut,
    ) -> Result<()> {
        if sealed.last_sequence != anchor.last_sequence
            || sealed.provider_cursor != anchor.provider_cursor
        {
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: self.expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: "observer rotation did not preserve an exact linked anchor".into(),
                command: "trail status".into(),
            });
        }
        let tx = Transaction::new_unchecked(&db.conn, TransactionBehavior::Immediate)?;
        let owner = self.observer.lease()?.owner_token;
        let now = crate::db::util::now_ts();
        let anchor_offset = i64::try_from(anchor.durable_end_offset)
            .map_err(|_| Error::InvalidInput("rotation anchor offset overflow".into()))?;
        let sealed_offset = i64::try_from(sealed.durable_end_offset)
            .map_err(|_| Error::InvalidInput("sealed offset overflow".into()))?;
        let epoch = i64::try_from(self.expected.epoch)
            .map_err(|_| Error::InvalidInput("rotation epoch overflow".into()))?;
        if sealed.segment_id == anchor.segment_id {
            let first_sequence = anchor
                .last_sequence
                .checked_add(1)
                .ok_or_else(|| Error::InvalidInput("rotation anchor sequence overflow".into()))?;
            let anchor_is_installed: bool = tx.query_row(
                "SELECT EXISTS(
                     SELECT 1
                     FROM changed_path_observer_segments segment
                     JOIN changed_path_scopes scope
                       ON scope.scope_id=segment.scope_id AND scope.epoch=segment.epoch
                     JOIN changed_path_observer_owners owner
                       ON owner.scope_id=scope.scope_id AND owner.epoch=scope.epoch
                     WHERE segment.scope_id=?1 AND segment.epoch=?2
                       AND segment.segment_id=?3 AND segment.owner_token=?4
                       AND segment.first_sequence=?5 AND segment.last_sequence IS NULL
                       AND segment.durable_end_offset=?6 AND segment.folded_end_offset=?6
                       AND segment.state='open'
                       AND scope.durable_offset=?6 AND scope.folded_offset=?6
                       AND scope.provider_cursor=?7 AND scope.observer_owner_token=?4
                       AND scope.trust_state='trusted'
                       AND owner.owner_token=?4 AND owner.lease_state='active'
                       AND owner.error_state IS NULL AND owner.expires_at>?8
                 )",
                params![
                    self.expected.scope_id.to_text(),
                    epoch,
                    anchor.segment_id,
                    owner,
                    i64::try_from(first_sequence)
                        .map_err(|_| Error::InvalidInput("rotation sequence overflow".into()))?,
                    anchor_offset,
                    anchor.provider_cursor,
                    now,
                ],
                |row| row.get(0),
            )?;
            if !anchor_is_installed {
                return Err(Error::ChangeLedgerReconcileRequired {
                    scope: self.expected.scope_id.to_text(),
                    state: "untrusted_gap".into(),
                    reason: "header-only observer rotation lost its exact installed anchor".into(),
                    command: "trail status".into(),
                });
            }
            tx.commit()?;
            return Ok(());
        }
        let sealed_segment_hash: String = tx.query_row(
            "SELECT segment_hash FROM changed_path_observer_segments
             WHERE scope_id=?1 AND epoch=?2 AND segment_id=?3 AND owner_token=?4
               AND COALESCE(last_sequence,0)=?5 AND durable_end_offset=?6 AND state='sealed'
               AND segment_hash IS NOT NULL",
            params![
                self.expected.scope_id.to_text(),
                epoch,
                sealed.segment_id,
                owner,
                i64::try_from(sealed.last_sequence)
                    .map_err(|_| Error::InvalidInput("sealed sequence overflow".into()))?,
                sealed_offset,
            ],
            |row| row.get(0),
        )?;
        let segment_changed = tx.execute(
            "UPDATE changed_path_observer_segments SET folded_end_offset=?1,updated_at=?2
             WHERE scope_id=?3 AND epoch=?4 AND segment_id=?5 AND owner_token=?6
               AND previous_segment_id=?7 AND previous_segment_hash=?8
               AND durable_end_offset>=?1 AND folded_end_offset=0 AND state='open'",
            params![
                anchor_offset,
                now,
                self.expected.scope_id.to_text(),
                epoch,
                anchor.segment_id,
                owner,
                sealed.segment_id,
                sealed_segment_hash,
            ],
        )?;
        let scope_changed = tx.execute(
            "UPDATE changed_path_scopes SET durable_offset=?1,folded_offset=?1,
                    provider_cursor=?2,trust_reason='observer_rotation_anchor',updated_at=?3
             WHERE scope_id=?4 AND epoch=?5 AND durable_offset=?6 AND folded_offset=?6
               AND observer_owner_token=?7 AND trust_state='trusted'",
            params![
                anchor_offset,
                anchor.provider_cursor,
                now,
                self.expected.scope_id.to_text(),
                epoch,
                sealed_offset,
                owner,
            ],
        )?;
        if segment_changed != 1 || scope_changed != 1 {
            let segment_state: Option<ObserverSegmentStateRow> = tx
                .query_row(
                    "SELECT durable_end_offset,folded_end_offset,previous_segment_id,
                            previous_segment_hash,state,owner_token
                     FROM changed_path_observer_segments
                     WHERE scope_id=?1 AND epoch=?2 AND segment_id=?3",
                    params![self.expected.scope_id.to_text(), epoch, anchor.segment_id],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                            row.get(5)?,
                        ))
                    },
                )
                .optional()?;
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: self.expected.scope_id.to_text(),
                state: "untrusted_gap".into(),
                reason: format!(
                    "observer rotation anchor lost its exact sidecar CAS (segment={segment_changed}, scope={scope_changed}, expected_anchor_offset={anchor_offset}, expected_previous_id={}, expected_previous_hash={sealed_segment_hash}, actual={segment_state:?})",
                    sealed.segment_id,
                ),
                command: "trail status".into(),
            });
        }
        tx.commit()?;
        Ok(())
    }

    fn current_proof(&self) -> Result<WorkspaceDaemonProof> {
        let lease = self.observer.lease().map_err(|error| {
            Error::DaemonUnavailable(format!(
                "changed-path observer health no longer authorizes the ready proof: {error}"
            ))
        })?;
        if !self.policy.observer_lease_matches(&self.expected, &lease)
            || !self.policy.authorizes_reconciliation(&self.expected)
        {
            return Err(Error::DaemonUnavailable(
                "changed-path observer health no longer authorizes the ready proof".into(),
            ));
        }
        Ok(WorkspaceDaemonProof {
            scope_id: self.expected.scope_id.to_text(),
            epoch: self.expected.epoch,
            observer_owner_token: lease.owner_token,
            daemon_launch_nonce: self.daemon_launch_nonce.clone(),
            cut: self.last_cut.clone().ok_or_else(|| {
                Error::DaemonUnavailable("workspace daemon has no authenticated ready cut".into())
            })?,
            reconcile_report: None,
        })
    }

    pub(crate) fn with_authoritative_snapshot<T, F>(
        &mut self,
        db: &Trail,
        consume: &mut F,
    ) -> Result<(T, FencedCandidateSnapshot)>
    where
        F: FnMut(&Trail, &CompiledPolicy, &super::CandidateSnapshot) -> Result<T>,
    {
        let mut retried = false;
        loop {
            let c1 = match self.fence(db) {
                Ok(proof) => proof.cut,
                Err(error)
                    if !retried
                        && requires_reconciliation(&error)
                        && !startup_policy_retryable(&error) =>
                {
                    self.reconcile(db, "authoritative_snapshot_retry")?;
                    retried = true;
                    continue;
                }
                Err(error) => return Err(error),
            };
            let mut candidates = match db.changed_path_ledger().snapshot_candidates(&self.expected)
            {
                Ok(snapshot) => snapshot,
                Err(error)
                    if !retried
                        && requires_reconciliation(&error)
                        && !startup_policy_retryable(&error) =>
                {
                    self.reconcile(db, "authoritative_snapshot_retry")?;
                    retried = true;
                    continue;
                }
                Err(error) => return Err(error),
            };
            candidates.cut = c1;
            let consumed = consume(db, &self.policy, &candidates);
            // c2 is mandatory even when comparison/build fails, so no command
            // can strand an unconsumed interval behind c1.
            let c2 = self.fence(db);
            match (consumed, c2) {
                (Ok(value), Ok(c2)) => {
                    return Ok((
                        value,
                        FencedCandidateSnapshot {
                            candidates,
                            c2: c2.cut,
                        },
                    ));
                }
                (Err(error), _)
                    if !retried
                        && requires_reconciliation(&error)
                        && !startup_policy_retryable(&error) =>
                {
                    self.reconcile(db, "authoritative_snapshot_retry")?;
                    retried = true;
                }
                (Err(error), _) => return Err(error),
                (_, Err(error))
                    if !retried
                        && requires_reconciliation(&error)
                        && !startup_policy_retryable(&error) =>
                {
                    self.reconcile(db, "authoritative_snapshot_retry")?;
                    retried = true;
                }
                (_, Err(error)) => return Err(error),
            }
        }
    }

    pub(crate) fn accept_observed_baseline(
        &mut self,
        expected: &ExpectedScope,
        target: &BaselineIdentity,
    ) -> Result<()> {
        // Post-commit repair may be retried after a caller loses the result.
        // Treat an already-rebound runtime as success, while still rejecting a
        // different target or scope.
        if self.expected.scope_id == expected.scope_id
            && self.expected.epoch == expected.epoch
            && self.expected.ref_name == target.ref_name
            && self.expected.ref_generation == target.ref_generation
            && self.expected.baseline_root == target.root_id
        {
            return Ok(());
        }
        if self.expected != *expected
            || self
                .last_cut
                .as_ref()
                .is_none_or(|cut| cut.durable_offset != cut.folded_offset)
        {
            return Err(Error::ChangeLedgerReconcileRequired {
                scope: expected.scope_id.to_text(),
                state: "stale_baseline".into(),
                reason: "daemon baseline transition did not match the committed observed scope"
                    .into(),
                command: "trail status".into(),
            });
        }
        let mut next = self.expected.clone();
        next.ref_name = target.ref_name.clone();
        next.ref_generation = target.ref_generation;
        next.baseline_root = target.root_id.clone();
        let anchor = self.tail_anchor.as_ref().ok_or_else(|| {
            Error::DaemonUnavailable("daemon has no retained tail to rebind".into())
        })?;
        self.observer
            .rebind_retained_tail(&self.expected, &next, anchor)?;
        self.policy
            .rebind_observed_baseline(&self.expected, &next)?;
        self.expected = next;
        Ok(())
    }
}

fn requires_reconciliation(error: &Error) -> bool {
    matches!(error, Error::ChangeLedgerReconcileRequired { .. })
}

#[cfg(debug_assertions)]
fn test_daemon_transition_after_load_boundary() -> Result<()> {
    let Some(barrier) = std::env::var_os("TRAIL_TEST_DAEMON_TRANSITION_AFTER_LOAD_BARRIER") else {
        return Ok(());
    };
    let barrier = std::path::PathBuf::from(barrier);
    fs::write(barrier.join("loaded"), b"ready")?;
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    while !barrier.join("continue").exists() {
        if std::time::Instant::now() >= deadline {
            return Err(Error::DaemonUnavailable(
                "daemon transition after-load test barrier timed out".into(),
            ));
        }
        std::thread::sleep(Duration::from_millis(1));
    }
    Ok(())
}

enum PlatformObserver {
    #[cfg(target_os = "linux")]
    Linux(super::observer::linux::LinuxInotifyObserver),
    #[cfg(target_os = "macos")]
    MacOs(super::observer::macos::MacOsFseventsObserver),
}

impl PlatformObserver {
    fn start(
        root: &Path,
        writer: SegmentWriter,
        provider_identity: Vec<u8>,
        fence_nonce: Vec<u8>,
        dependencies: &[std::path::PathBuf],
        resume_cursor: Option<Vec<u8>>,
    ) -> Result<Self> {
        #[cfg(target_os = "linux")]
        {
            let _ = resume_cursor;
            let durability = super::observer::linux::SegmentWriterDurability::new(
                writer,
                provider_identity,
                fence_nonce,
            )?;
            return Ok(Self::Linux(
                super::observer::linux::LinuxInotifyObserver::start(
                    root,
                    Box::new(durability),
                    dependencies,
                )?,
            ));
        }
        #[cfg(target_os = "macos")]
        {
            let resume = resume_cursor
                .as_deref()
                .map(super::observer::macos::MacOsProviderCursor::decode)
                .transpose()?;
            let durability = super::observer::macos::MacSegmentWriterDurability::new(
                writer,
                provider_identity,
                fence_nonce,
            )?;
            Ok(Self::MacOs(
                super::observer::macos::MacOsFseventsObserver::start(
                    root,
                    Box::new(durability),
                    resume,
                    dependencies,
                )?,
            ))
        }
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        {
            let _ = (
                root,
                writer,
                provider_identity,
                fence_nonce,
                dependencies,
                resume_cursor,
            );
            Err(Error::DaemonUnavailable(
                "changed-path workspace daemon requires Linux or macOS".into(),
            ))
        }
    }

    fn lease(&self) -> Result<ObserverLease> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(observer) => observer.lease(),
            #[cfg(target_os = "macos")]
            Self::MacOs(observer) => observer.lease(),
        }
    }

    #[cfg(all(test, target_os = "macos"))]
    fn fail_next_direct_policy_fence_for_test(&self) {
        match self {
            Self::MacOs(observer) => observer.fail_next_direct_policy_fence_for_test(),
        }
    }

    fn seal_after_fence(
        &self,
        expected: &ExpectedScope,
        fence: &ObserverFence,
    ) -> Result<Option<(DurableCut, DurableCut)>> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(observer) => observer.seal_after_fence(expected, fence),
            #[cfg(target_os = "macos")]
            Self::MacOs(observer) => observer.seal_after_fence(expected, fence),
        }
    }

    fn controlled_end_fence(
        &self,
        expected: &ExpectedScope,
        start: &ObserverFence,
    ) -> Result<(ObserverFence, DurableCut, DurableCut)> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(observer) => observer.controlled_end_fence(expected, start),
            #[cfg(target_os = "macos")]
            Self::MacOs(observer) => observer.controlled_end_fence(expected, start),
        }
    }

    fn install_rotation_anchor(
        &self,
        expected: &ExpectedScope,
        end: &ObserverFence,
        anchor: DurableCut,
    ) -> Result<ObserverFence> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(observer) => observer.install_rotation_anchor(expected, end, anchor),
            #[cfg(target_os = "macos")]
            Self::MacOs(observer) => observer.install_rotation_anchor(expected, end, anchor),
        }
    }

    fn authenticated_cut(
        &self,
        expected: &ExpectedScope,
        fence: &ObserverFence,
    ) -> Result<DurableCut> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(observer) => observer.authenticated_cut(expected, fence),
            #[cfg(target_os = "macos")]
            Self::MacOs(observer) => observer.authenticated_cut(expected, fence),
        }
    }
}

impl QualifiedObserver for PlatformObserver {
    fn begin_observation(&self, expected: &ExpectedScope) -> Result<ObserverFence> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(observer) => observer.begin_observation(expected),
            #[cfg(target_os = "macos")]
            Self::MacOs(observer) => observer.begin_observation(expected),
        }
    }

    fn end_fence(&self, expected: &ExpectedScope, start: &ObserverFence) -> Result<ObserverFence> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(observer) => observer.end_fence(expected, start),
            #[cfg(target_os = "macos")]
            Self::MacOs(observer) => observer.end_fence(expected, start),
        }
    }

    fn drain_through(
        &self,
        expected: &ExpectedScope,
        root_handle_identity: &[u8],
        start: &ObserverFence,
        end: &ObserverFence,
        sink: &mut dyn FnMut(super::ObserverEvent) -> Result<()>,
    ) -> Result<super::reconcile::ObserverQualification> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(observer) => {
                observer.drain_through(expected, root_handle_identity, start, end, sink)
            }
            #[cfg(target_os = "macos")]
            Self::MacOs(observer) => {
                observer.drain_through(expected, root_handle_identity, start, end, sink)
            }
        }
    }

    fn drain_through_retaining_end(
        &self,
        expected: &ExpectedScope,
        root_handle_identity: &[u8],
        start: &ObserverFence,
        end: &ObserverFence,
        sink: &mut dyn FnMut(super::ObserverEvent) -> Result<()>,
    ) -> Result<super::reconcile::ObserverQualification> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(observer) => observer.drain_through_retaining_end(
                expected,
                root_handle_identity,
                start,
                end,
                sink,
            ),
            #[cfg(target_os = "macos")]
            Self::MacOs(observer) => observer.drain_through_retaining_end(
                expected,
                root_handle_identity,
                start,
                end,
                sink,
            ),
        }
    }

    fn rebind_retained_tail(
        &self,
        previous: &ExpectedScope,
        next: &ExpectedScope,
        anchor: &ObserverFence,
    ) -> Result<()> {
        match self {
            #[cfg(target_os = "linux")]
            Self::Linux(observer) => observer.rebind_retained_tail(previous, next, anchor),
            #[cfg(target_os = "macos")]
            Self::MacOs(observer) => observer.rebind_retained_tail(previous, next, anchor),
        }
    }
}

fn workspace_scope_id(db: &Trail) -> ScopeId {
    let mut digest = Sha256::new();
    digest.update(b"trail-workspace-changed-path-scope-v1\0");
    digest.update(db.config.workspace.id.0.as_bytes());
    ScopeId(digest.finalize().into())
}

fn workspace_daemon_target(db: &Trail) -> Result<DaemonScopeTarget> {
    let branch = db.current_branch()?;
    let head = db.resolve_branch_ref(&branch)?;
    Ok(DaemonScopeTarget {
        root: db.workspace_root().to_path_buf(),
        identity: ScopeIdentity {
            scope_id: workspace_scope_id(db),
            kind: ScopeKind::Workspace,
            owner_id: db.config.workspace.id.0.clone(),
        },
        baseline: BaselineIdentity {
            ref_name: head.name,
            ref_generation: u64::try_from(head.generation)
                .map_err(|_| Error::Corrupt("negative daemon ref generation".into()))?,
            change_id: head.change_id,
            root_id: head.root_id,
        },
    })
}

fn materialized_lane_daemon_target(db: &Trail, lane: &str) -> Result<DaemonScopeTarget> {
    let branch = db.lane_branch(lane)?;
    let workdir = branch.workdir.as_deref().ok_or_else(|| {
        Error::InvalidInput(format!(
            "lane `{}` does not have a materialized workdir",
            branch.lane_id
        ))
    })?;
    let root = std::path::PathBuf::from(workdir);
    if !root.is_dir() {
        return Err(Error::InvalidInput(format!(
            "materialized lane workdir `{}` is unavailable",
            root.display()
        )));
    }
    let head = db.get_ref(&branch.ref_name)?;
    if head.change_id != branch.head_change || head.root_id != branch.head_root {
        return Err(Error::StaleBranch(branch.ref_name));
    }
    let scope_id = materialized_lane_scope_id(&db.config.workspace.id.0, &branch.lane_id);
    Ok(DaemonScopeTarget {
        root,
        identity: ScopeIdentity {
            scope_id,
            kind: ScopeKind::MaterializedLane,
            owner_id: branch.lane_id,
        },
        baseline: BaselineIdentity {
            ref_name: head.name,
            ref_generation: u64::try_from(head.generation)
                .map_err(|_| Error::Corrupt("negative lane ref generation".into()))?,
            change_id: head.change_id,
            root_id: head.root_id,
        },
    })
}

pub(crate) fn materialized_lane_scope_id(workspace_id: &str, lane_id: &str) -> ScopeId {
    let mut digest = Sha256::new();
    digest.update(b"trail-materialized-lane-changed-path-scope-v2\0");
    digest.update(workspace_id.as_bytes());
    digest.update([0]);
    digest.update(lane_id.as_bytes());
    ScopeId(digest.finalize().into())
}

fn daemon_authority_transition_lost(scope_id: ScopeId) -> Error {
    Error::ChangeLedgerReconcileRequired {
        scope: scope_id.to_text(),
        state: "stale_baseline".into(),
        reason: "daemon authority transition lost exact loaded authority".into(),
        command: "trail status".into(),
    }
}

fn daemon_owner_authority_inconsistent(scope_id: ScopeId) -> Error {
    Error::ChangeLedgerReconcileRequired {
        scope: scope_id.to_text(),
        state: "corrupt".into(),
        reason: "persisted daemon scope and observer owner authority are inconsistent".into(),
        command: "trail status".into(),
    }
}

fn platform_provider_identity() -> Vec<u8> {
    #[cfg(target_os = "linux")]
    return b"linux-inotify-native-v1".to_vec();
    #[cfg(target_os = "macos")]
    return b"macos-fsevents-segment-writer-v1".to_vec();
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    Vec::new()
}

fn platform_capabilities() -> ProviderCapabilities {
    ProviderCapabilities {
        durable_cursor: true,
        linearizable_fence: true,
        rename_pairing: cfg!(target_os = "linux"),
        overflow_scope: true,
        filesystem_supported: cfg!(any(target_os = "linux", target_os = "macos")),
        clean_proof_allowed: cfg!(any(target_os = "linux", target_os = "macos")),
        power_loss_durability: true,
    }
}

fn root_identity(path: &Path) -> Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?;
    let metadata = file.metadata()?;
    let prefix = "root-v1";
    Ok(format!(
        "{prefix}:dev={};ino={};mode={};uid={};gid={}",
        metadata.dev(),
        metadata.ino(),
        metadata.mode(),
        metadata.uid(),
        metadata.gid()
    )
    .into_bytes())
}

fn decode_fingerprint(value: &str) -> Result<[u8; 32]> {
    let bytes = hex::decode(value)
        .map_err(|_| Error::Corrupt("invalid changed-path policy fingerprint".into()))?;
    bytes
        .try_into()
        .map_err(|_| Error::Corrupt("invalid changed-path policy fingerprint length".into()))
}

#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    use super::*;

    #[test]
    fn publication_header_anchor_accepts_only_its_own_contiguous_later_suffix() {
        let publication_cut = EvidenceCut {
            source: EvidenceSource::Observer,
            sequence: 7,
            durable_offset: 101,
            folded_offset: 101,
        };
        let publication_durable = DurableCut {
            segment_id: "publication-segment".into(),
            durable_end_offset: 101,
            last_sequence: 7,
            last_hash: [7; 32],
            provider_cursor: b"cursor-7".to_vec(),
        };
        let segment = |segment_id: &str| super::super::AuthenticatedSegment {
            segment_id: segment_id.into(),
            segment_path: format!("{segment_id}.wal"),
            state: "open".into(),
            start_cursor: b"cursor-7".to_vec(),
            end_cursor: b"cursor-8".to_vec(),
            first_sequence: 8,
            last_sequence: 8,
            header_end_offset: 101,
            durable_end_offset: 180,
            folded_end_offset: 101,
            segment_hash: [8; 32],
        };
        let recovered = RecoveredTail {
            records: Vec::new(),
            record_boundaries: Vec::new(),
            durable_end: 360,
            last_sequence: 8,
            last_hash: [8; 32],
            requires_reconciliation: false,
            segments: vec![segment("decoy-segment"), segment("publication-segment")],
        };

        assert_eq!(
            authenticated_publication_boundary(&recovered, &publication_cut, &publication_durable,),
            Some(("publication-segment".into(), b"cursor-7".to_vec(),))
        );
    }

    #[test]
    fn explicit_full_reconcile_repairs_an_existing_runtime_without_a_ready_proof() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("tracked.txt"), b"baseline\n").unwrap();
        crate::Trail::init(
            temp.path(),
            "main",
            crate::InitImportMode::WorkingTree,
            false,
        )
        .unwrap();
        let db = crate::Trail::open(temp.path()).unwrap();
        prepare_workspace_daemon(&db, true).unwrap();

        daemon_registry(&db).workspace.as_mut().unwrap().last_cut = None;
        assert!(workspace_daemon_ready_proof(&db).is_err());

        let report = workspace_daemon_full_reconcile(&db).unwrap();
        assert_eq!(report.scope_kind, "workspace");
        assert_eq!(report.resulting_state, "trusted");
        assert!(workspace_daemon_ready_proof(&db).is_ok());
    }

    #[test]
    fn repeated_controlled_preparation_reuses_an_authenticated_header_only_anchor() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("tracked.txt"), b"baseline\n").unwrap();
        crate::Trail::init(
            temp.path(),
            "main",
            crate::InitImportMode::WorkingTree,
            false,
        )
        .unwrap();
        let mut db = crate::Trail::open(temp.path()).unwrap();

        let first = prepare_workspace_controlled_projection(&mut db).unwrap();
        let second = prepare_workspace_controlled_projection(&mut db).unwrap();
        let third = prepare_workspace_controlled_projection(&mut db).unwrap();
        assert_eq!(first.scope_id, second.scope_id);
        assert_eq!(second.scope_id, third.scope_id);
        assert_eq!(first.epoch, second.epoch);
        assert_eq!(second.epoch, third.epoch);
        assert!(workspace_daemon_ready_proof(&db).is_ok());
    }

    #[test]
    fn initial_materialized_lane_policy_invalidation_reconciles_and_retries_transparently() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("tracked.txt"), b"baseline\n").unwrap();
        crate::Trail::init(
            temp.path(),
            "main",
            crate::InitImportMode::WorkingTree,
            false,
        )
        .unwrap();
        let mut db = crate::Trail::open(temp.path()).unwrap();
        let lane = db
            .spawn_lane("startup-policy-retry", Some("main"), true, None, None)
            .unwrap();
        let lane_id = lane.lane_id;
        let workdir = std::path::PathBuf::from(lane.workdir.unwrap());
        let scope_id = materialized_lane_scope_id(&db.config.workspace.id.0, &lane_id);
        super::super::install_initial_scan_hook(scope_id, move || {
            std::fs::write(workdir.join(".gitignore"), b"generated/**\n")?;
            Ok(())
        });

        let report = materialized_lane_daemon_full_reconcile(&db, &lane_id).unwrap();
        assert_eq!(report.scope_kind, "materialized_lane");
        assert_eq!(report.resulting_state, "trusted");
        assert_eq!(report.resulting_epoch, 2);
        assert!(materialized_lane_daemon_ready_proof(&db, &lane_id).is_ok());
    }

    #[test]
    fn initial_workspace_policy_invalidation_retries_with_the_same_daemon_launch() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("tracked.txt"), b"baseline\n").unwrap();
        crate::Trail::init(
            temp.path(),
            "main",
            crate::InitImportMode::WorkingTree,
            false,
        )
        .unwrap();
        let db = crate::Trail::open(temp.path()).unwrap();
        let scope_id = workspace_daemon_target(&db).unwrap().identity.scope_id;
        let root = temp.path().to_path_buf();
        super::super::install_initial_scan_hook(scope_id, move || {
            std::fs::write(root.join(".gitignore"), b"generated/**\n")?;
            Ok(())
        });
        let launch = WorkspaceDaemonLaunchIdentity {
            nonce: "9a".repeat(32),
            pid: std::process::id(),
            process_start_identity: "workspace-startup-retry-test".into(),
        };

        let proof = prepare_workspace_daemon_launch(&db, launch.clone(), None).unwrap();
        assert_eq!(proof.epoch, 2);
        assert_eq!(proof.daemon_launch_nonce, Some(launch.nonce.clone()));
        assert_eq!(proof.reconcile_report.unwrap().resulting_state, "trusted");
        assert!(workspace_daemon_ready_proof(&db).is_ok());
        let binding: (i64, String, i64, String, String) = db
            .conn
            .query_row(
                "SELECT epoch,daemon_launch_nonce,daemon_pid,
                        daemon_process_start_identity,lease_state
                 FROM changed_path_observer_owners WHERE scope_id=?1",
                [scope_id.to_text()],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(binding.0, 2);
        assert_eq!(binding.1, launch.nonce);
        assert_eq!(binding.2, i64::from(launch.pid));
        assert_eq!(binding.3, launch.process_start_identity);
        assert_eq!(binding.4, "active");
    }

    #[test]
    fn workspace_startup_retry_cannot_replace_an_owner_swapped_after_capability_capture() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("tracked.txt"), b"baseline\n").unwrap();
        crate::Trail::init(
            temp.path(),
            "main",
            crate::InitImportMode::WorkingTree,
            false,
        )
        .unwrap();
        let db = crate::Trail::open(temp.path()).unwrap();
        let scope_id = workspace_daemon_target(&db).unwrap().identity.scope_id;
        let root = temp.path().to_path_buf();
        super::super::install_initial_scan_hook(scope_id, move || {
            std::fs::write(root.join(".gitignore"), b"generated/**\n")?;
            Ok(())
        });
        let replacement_token = "bc".repeat(32);
        let replacement_nonce = "cd".repeat(32);
        let expected_token = replacement_token.clone();
        let expected_nonce = replacement_nonce.clone();
        install_workspace_retry_boundary_hook(scope_id, move |db| {
            let tx = db.conn.unchecked_transaction()?;
            let owner_changed = tx.execute(
                "UPDATE changed_path_observer_owners
                 SET owner_token=?1,daemon_launch_nonce=?2,daemon_pid=?3,
                     daemon_process_start_identity='different-live-owner',
                     lease_state='active',error_state=NULL,error_at=NULL,
                     heartbeat_at=strftime('%s','now'),expires_at=strftime('%s','now')+30,
                     updated_at=strftime('%s','now')
                 WHERE scope_id=?4",
                params![
                    replacement_token,
                    replacement_nonce,
                    i64::from(std::process::id()),
                    scope_id.to_text(),
                ],
            )?;
            let scope_changed = tx.execute(
                "UPDATE changed_path_scopes SET observer_owner_token=?1 WHERE scope_id=?2",
                params![replacement_token, scope_id.to_text()],
            )?;
            if owner_changed != 1 || scope_changed != 1 {
                return Err(Error::Corrupt(
                    "retry-race fixture could not install replacement owner".into(),
                ));
            }
            tx.commit()?;
            Ok(())
        });
        let launch = WorkspaceDaemonLaunchIdentity {
            nonce: "ab".repeat(32),
            pid: std::process::id(),
            process_start_identity: "original-startup-owner".into(),
        };

        let error = prepare_workspace_daemon_launch(&db, launch, None)
            .err()
            .expect("swapped owner unexpectedly authorized retry");
        assert!(error
            .to_string()
            .contains("does not match the exact persisted observer scope/epoch/owner token"));
        assert!(daemon_registry(&db).workspace.is_none());
        let owner: (String, String, String, String) = db
            .conn
            .query_row(
                "SELECT owner.owner_token,owner.daemon_launch_nonce,owner.lease_state,
                        scope.observer_owner_token
                 FROM changed_path_observer_owners owner
                 JOIN changed_path_scopes scope ON scope.scope_id=owner.scope_id
                 WHERE owner.scope_id=?1",
                [scope_id.to_text()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            owner,
            (
                expected_token.clone(),
                expected_nonce,
                "active".into(),
                expected_token
            )
        );
    }

    #[test]
    fn startup_retry_classification_keeps_overflow_fail_closed() {
        assert!(startup_policy_retryable(
            &Error::ChangeLedgerReconcileRequired {
                scope: "scope".into(),
                state: "stale_baseline".into(),
                reason: "fsevents_policy_dependency_invalidated:.gitignore".into(),
                command: "trail index reconcile".into(),
            }
        ));
        assert!(!startup_policy_retryable(
            &Error::ChangeLedgerReconcileRequired {
                scope: "scope".into(),
                state: "untrusted_gap".into(),
                reason: "fsevents_history_overflow".into(),
                command: "trail index reconcile".into(),
            }
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn direct_policy_drift_between_end_fence_and_lease_restarts_and_reconciles() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("tracked.txt"), b"baseline\n").unwrap();
        crate::Trail::init(
            temp.path(),
            "main",
            crate::InitImportMode::WorkingTree,
            false,
        )
        .unwrap();
        let db = crate::Trail::open(temp.path()).unwrap();
        let first = prepare_workspace_daemon(&db, true).unwrap();
        daemon_registry(&db)
            .workspace
            .as_mut()
            .unwrap()
            .inject_policy_drift_after_end = true;

        let restarted = workspace_daemon_fence(&db, None, None).unwrap();

        assert!(restarted.epoch > first.epoch);
        assert_eq!(
            restarted.reconcile_report.unwrap().resulting_state,
            "trusted"
        );
        assert!(workspace_daemon_ready_proof(&db).is_ok());
    }

    #[test]
    fn persistent_startup_policy_churn_exhausts_the_bound_and_fails_closed() {
        fn install_churn(scope_id: ScopeId, ignore: std::path::PathBuf, remaining: usize) {
            super::super::install_initial_scan_hook(scope_id, move || {
                std::fs::write(&ignore, format!("generated-{remaining}/**\n").as_bytes())?;
                if remaining > 1 {
                    install_churn(scope_id, ignore, remaining - 1);
                }
                Ok(())
            });
        }

        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("tracked.txt"), b"baseline\n").unwrap();
        crate::Trail::init(
            temp.path(),
            "main",
            crate::InitImportMode::WorkingTree,
            false,
        )
        .unwrap();
        let mut db = crate::Trail::open(temp.path()).unwrap();
        let lane = db
            .spawn_lane("persistent-policy-churn", Some("main"), true, None, None)
            .unwrap();
        let lane_id = lane.lane_id;
        let scope_id = materialized_lane_scope_id(&db.config.workspace.id.0, &lane_id);
        install_churn(
            scope_id,
            std::path::PathBuf::from(lane.workdir.unwrap()).join(".gitignore"),
            3,
        );

        let error = materialized_lane_daemon_full_reconcile(&db, &lane_id).unwrap_err();
        assert!(
            matches!(&error, Error::ChangeLedgerReconcileRequired { .. }),
            "persistent churn failed with the wrong error: {error:?}"
        );
        assert!(!daemon_registry(&db)
            .materialized_lanes
            .contains_key(&lane_id));
        let state: String = db
            .conn
            .query_row(
                "SELECT trust_state FROM changed_path_scopes WHERE scope_id=?1",
                [scope_id.to_text()],
                |row| row.get(0),
            )
            .unwrap();
        assert_ne!(state, "trusted");
    }
}

struct ExistingScope {
    scope_kind: String,
    owner_id: String,
    scope_root: String,
    scope_root_identity: String,
    filesystem_kind: String,
    case_sensitive: i64,
    schema_version: i64,
    limits: [i64; 5],
    epoch: u64,
    ref_name: String,
    ref_generation: u64,
    change_id: String,
    baseline_root: String,
    policy_fingerprint: [u8; 32],
    policy_generation: u64,
    filesystem_identity: Vec<u8>,
    provider_identity: Vec<u8>,
    provider_id_text: Option<String>,
    provider_identity_text: Option<String>,
    provider_cursor: Option<Vec<u8>>,
    provider_fence: Option<Vec<u8>>,
    capabilities: [i64; 7],
    trust_state: String,
    trust_reason: String,
    continuity_generation: u64,
    durable_offset: u64,
    folded_offset: u64,
    observer_owner_token: Option<String>,
    observer_heartbeat_at: Option<i64>,
    observer_error_state: Option<String>,
    observer_error_at: Option<i64>,
    retired_at: Option<i64>,
    observer_owner: Option<ExistingObserverOwner>,
}

struct ExistingObserverOwner {
    epoch: u64,
    owner_token: String,
    provider_id: String,
    provider_identity: String,
    lease_state: String,
    fence_nonce: Option<Vec<u8>>,
    acquired_at: i64,
    heartbeat_at: i64,
    expires_at: i64,
    error_state: Option<String>,
    error_at: Option<i64>,
    daemon_launch_nonce: Option<String>,
    daemon_pid: Option<i64>,
    daemon_process_start_identity: Option<String>,
}

fn load_existing_scope(db: &Trail, scope_id: ScopeId) -> Result<Option<ExistingScope>> {
    let row = db
        .conn
        .query_row(
            "SELECT scope.scope_kind,scope.owner_id,scope.scope_root,
                    scope.scope_root_identity,scope.filesystem_kind,scope.case_sensitive,
                    scope.epoch,scope.ref_name,scope.ref_generation,scope.change_id,
                    scope.baseline_root_id,scope.policy_fingerprint,
                    scope.policy_dependency_generation,scope.filesystem_identity,
                    scope.provider_id,scope.provider_identity,scope.provider_cursor,
                    scope.provider_fence,scope.durable_cursor,scope.linearizable_fence,
                    scope.rename_pairing,scope.overflow_scope,scope.filesystem_supported,
                    scope.clean_proof_allowed,scope.power_loss_durability,
                    scope.trust_state,scope.trust_reason,scope.continuity_generation,
                    scope.durable_offset,scope.folded_offset,scope.observer_owner_token,
                    scope.observer_heartbeat_at,scope.observer_error_state,
                    scope.observer_error_at,scope.retired_at,scope.schema_version,
                    scope.max_candidate_rows,scope.max_prefix_rows,
                    scope.max_observer_log_bytes,scope.max_segment_bytes,
                    scope.max_unfolded_tail_records,
                    owner.epoch,owner.owner_token,owner.provider_id,
                    owner.provider_identity,owner.lease_state,owner.fence_nonce,
                    owner.acquired_at,owner.heartbeat_at,owner.expires_at,
                    owner.error_state,owner.error_at,owner.daemon_launch_nonce,
                    owner.daemon_pid,owner.daemon_process_start_identity
             FROM changed_path_scopes scope
             LEFT JOIN changed_path_observer_owners owner ON owner.scope_id=scope.scope_id
             WHERE scope.scope_id=?1",
            [scope_id.to_text()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, String>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, String>(11)?,
                    row.get::<_, i64>(12)?,
                    row.get::<_, String>(13)?,
                    row.get::<_, Option<String>>(14)?,
                    row.get::<_, Option<String>>(15)?,
                    row.get::<_, Option<Vec<u8>>>(16)?,
                    row.get::<_, Option<Vec<u8>>>(17)?,
                    [
                        row.get::<_, i64>(18)?,
                        row.get::<_, i64>(19)?,
                        row.get::<_, i64>(20)?,
                        row.get::<_, i64>(21)?,
                        row.get::<_, i64>(22)?,
                        row.get::<_, i64>(23)?,
                        row.get::<_, i64>(24)?,
                    ],
                    row.get::<_, String>(25)?,
                    row.get::<_, String>(26)?,
                    row.get::<_, i64>(27)?,
                    row.get::<_, i64>(28)?,
                    row.get::<_, i64>(29)?,
                    row.get::<_, Option<String>>(30)?,
                    row.get::<_, Option<i64>>(31)?,
                    row.get::<_, Option<String>>(32)?,
                    row.get::<_, Option<i64>>(33)?,
                    row.get::<_, Option<i64>>(34)?,
                    row.get::<_, i64>(35)?,
                    [
                        row.get::<_, i64>(36)?,
                        row.get::<_, i64>(37)?,
                        row.get::<_, i64>(38)?,
                        row.get::<_, i64>(39)?,
                        row.get::<_, i64>(40)?,
                    ],
                    row.get::<_, Option<i64>>(41)?,
                    row.get::<_, Option<String>>(42)?,
                    row.get::<_, Option<String>>(43)?,
                    row.get::<_, Option<String>>(44)?,
                    row.get::<_, Option<String>>(45)?,
                    row.get::<_, Option<Vec<u8>>>(46)?,
                    row.get::<_, Option<i64>>(47)?,
                    row.get::<_, Option<i64>>(48)?,
                    row.get::<_, Option<i64>>(49)?,
                    row.get::<_, Option<String>>(50)?,
                    row.get::<_, Option<i64>>(51)?,
                    row.get::<_, Option<String>>(52)?,
                    row.get::<_, Option<i64>>(53)?,
                    row.get::<_, Option<String>>(54)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(
            scope_kind,
            owner_id,
            scope_root,
            scope_root_identity,
            filesystem_kind,
            case_sensitive,
            epoch,
            ref_name,
            ref_generation,
            change_id,
            baseline_root,
            policy_fingerprint,
            policy_generation,
            filesystem_identity,
            provider_id_text,
            provider_identity_text,
            provider_cursor,
            provider_fence,
            capabilities,
            trust_state,
            trust_reason,
            continuity_generation,
            durable_offset,
            folded_offset,
            observer_owner_token,
            observer_heartbeat_at,
            observer_error_state,
            observer_error_at,
            retired_at,
            schema_version,
            limits,
            owner_epoch,
            owner_token,
            owner_provider_id,
            owner_provider_identity,
            owner_lease_state,
            owner_fence_nonce,
            owner_acquired_at,
            owner_heartbeat_at,
            owner_expires_at,
            owner_error_state,
            owner_error_at,
            owner_daemon_launch_nonce,
            owner_daemon_pid,
            owner_daemon_process_start_identity,
        )| {
            let provider_identity_text = provider_identity_text.ok_or_else(|| {
                Error::Corrupt("daemon scope is missing provider identity".into())
            })?;
            let observer_owner = match (
                owner_epoch,
                owner_token,
                owner_provider_id,
                owner_provider_identity,
                owner_lease_state,
                owner_acquired_at,
                owner_heartbeat_at,
                owner_expires_at,
            ) {
                (
                    Some(epoch),
                    Some(token),
                    Some(provider_id),
                    Some(provider_identity),
                    Some(lease_state),
                    Some(acquired_at),
                    Some(heartbeat_at),
                    Some(expires_at),
                ) => Some(ExistingObserverOwner {
                    epoch: u64::try_from(epoch)
                        .map_err(|_| Error::Corrupt("negative observer owner epoch".into()))?,
                    owner_token: token,
                    provider_id,
                    provider_identity,
                    lease_state,
                    fence_nonce: owner_fence_nonce,
                    acquired_at,
                    heartbeat_at,
                    expires_at,
                    error_state: owner_error_state,
                    error_at: owner_error_at,
                    daemon_launch_nonce: owner_daemon_launch_nonce,
                    daemon_pid: owner_daemon_pid,
                    daemon_process_start_identity: owner_daemon_process_start_identity,
                }),
                (None, None, None, None, None, None, None, None) => None,
                _ => {
                    return Err(Error::Corrupt(
                        "partial daemon observer owner authority".into(),
                    ))
                }
            };
            Ok(ExistingScope {
                scope_kind,
                owner_id,
                scope_root,
                scope_root_identity,
                filesystem_kind,
                case_sensitive,
                schema_version,
                limits,
                epoch: u64::try_from(epoch)
                    .map_err(|_| Error::Corrupt("negative changed-path scope epoch".into()))?,
                ref_name,
                ref_generation: u64::try_from(ref_generation)
                    .map_err(|_| Error::Corrupt("negative changed-path ref generation".into()))?,
                change_id,
                baseline_root,
                policy_fingerprint: decode_fingerprint(&policy_fingerprint)?,
                policy_generation: u64::try_from(policy_generation).map_err(|_| {
                    Error::Corrupt("negative changed-path policy generation".into())
                })?,
                filesystem_identity: hex::decode(filesystem_identity).map_err(|_| {
                    Error::Corrupt("invalid changed-path filesystem identity".into())
                })?,
                provider_identity: hex::decode(&provider_identity_text)
                    .map_err(|_| Error::Corrupt("invalid changed-path provider identity".into()))?,
                provider_id_text,
                provider_identity_text: Some(provider_identity_text),
                provider_cursor,
                provider_fence,
                capabilities,
                trust_state,
                trust_reason,
                continuity_generation: u64::try_from(continuity_generation)
                    .map_err(|_| Error::Corrupt("negative continuity generation".into()))?,
                durable_offset: u64::try_from(durable_offset)
                    .map_err(|_| Error::Corrupt("negative durable offset".into()))?,
                folded_offset: u64::try_from(folded_offset)
                    .map_err(|_| Error::Corrupt("negative folded offset".into()))?,
                observer_owner_token,
                observer_heartbeat_at,
                observer_error_state,
                observer_error_at,
                retired_at,
                observer_owner,
            })
        },
    )
    .transpose()
}
