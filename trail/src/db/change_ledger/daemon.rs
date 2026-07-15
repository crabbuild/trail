use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::Duration;

use getrandom::getrandom;
use rusqlite::{named_params, params, OptionalExtension};
use sha2::{Digest, Sha256};

use super::{
    compile_policy, reconcile_full, BaselineIdentity, CompiledPolicy, EvidenceCut, EvidenceSource,
    ExpectedScope, FilesystemIdentity, ObserverFence, ObserverLease, PolicyCompileContext,
    PolicyDependencyMetrics, PolicyIdentity, ProviderCapabilities, ProviderIdentity,
    QualifiedObserver, ScopeId, ScopeIdentity, ScopeKind, SegmentWriter,
};
use crate::error::{Error, Result};
use crate::Trail;

pub(crate) struct WorkspaceDaemonProof {
    pub(crate) scope_id: String,
    pub(crate) epoch: u64,
    pub(crate) cut: EvidenceCut,
}

pub(crate) fn prepare_workspace_daemon(
    db: &mut Trail,
    replace_verified_stale_owner: bool,
) -> Result<WorkspaceDaemonProof> {
    if db.changed_path_daemon_runtime.is_some() {
        return workspace_daemon_fence(db, None, None);
    }
    let mut runtime = WorkspaceDaemonRuntime::start(db, replace_verified_stale_owner)?;
    let proof = runtime.reconcile(db, "daemon_initial_full_reconciliation")?;
    db.changed_path_daemon_runtime = Some(runtime);
    Ok(proof)
}

pub(crate) fn workspace_daemon_fence(
    db: &mut Trail,
    scope_id: Option<&str>,
    epoch: Option<u64>,
) -> Result<WorkspaceDaemonProof> {
    let mut runtime = db.changed_path_daemon_runtime.take().ok_or_else(|| {
        Error::DaemonUnavailable("changed-path observer runtime is unavailable".into())
    })?;
    runtime.validate_request(scope_id, epoch)?;
    let result = runtime.fence(db);
    db.changed_path_daemon_runtime = Some(runtime);
    result
}

pub(crate) fn workspace_daemon_reconcile(
    db: &mut Trail,
    scope_id: Option<&str>,
    epoch: Option<u64>,
) -> Result<WorkspaceDaemonProof> {
    let mut runtime = db.changed_path_daemon_runtime.take().ok_or_else(|| {
        Error::DaemonUnavailable("changed-path observer runtime is unavailable".into())
    })?;
    runtime.validate_request(scope_id, epoch)?;
    let result = runtime.reconcile(db, "daemon_requested_full_reconciliation");
    db.changed_path_daemon_runtime = Some(runtime);
    result
}

pub(crate) fn workspace_daemon_ready_proof(db: &Trail) -> Result<WorkspaceDaemonProof> {
    db.changed_path_daemon_runtime
        .as_ref()
        .ok_or_else(|| {
            Error::DaemonUnavailable("changed-path observer runtime is unavailable".into())
        })?
        .current_proof()
}

pub(crate) struct WorkspaceDaemonRuntime {
    expected: ExpectedScope,
    policy: CompiledPolicy,
    observer: PlatformObserver,
    last_cut: Option<EvidenceCut>,
}

impl WorkspaceDaemonRuntime {
    fn start(db: &Trail, replace_verified_stale_owner: bool) -> Result<Self> {
        let branch = db.current_branch()?;
        let head = db.resolve_branch_ref(&branch)?;
        let scope_id = workspace_scope_id(db);
        let segment_directory = db.db_dir.join("observer-segments").join(scope_id.to_text());
        fs::create_dir_all(&segment_directory)?;
        let filesystem_identity = root_identity(db.workspace_root())?;
        let provider_identity = platform_provider_identity();
        let capabilities = platform_capabilities();
        let baseline = BaselineIdentity {
            ref_name: head.name,
            ref_generation: u64::try_from(head.generation)
                .map_err(|_| Error::Corrupt("negative daemon ref generation".into()))?,
            change_id: head.change_id,
            root_id: head.root_id,
        };
        let ledger = db.changed_path_ledger();
        let existing = load_existing_scope(db, scope_id)?;
        #[cfg(debug_assertions)]
        if existing.is_some() {
            test_daemon_transition_after_load_boundary()?;
        }
        let (epoch, mut resume_cursor) = match existing {
            None => {
                ledger.begin_scope(
                    &ScopeIdentity {
                        scope_id,
                        kind: ScopeKind::Workspace,
                        owner_id: db.config.workspace.id.0.clone(),
                    },
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
                )?;
                (1, None)
            }
            Some(stored) => {
                let current_filesystem_identity = hex::encode(&filesystem_identity);
                let current_provider_identity = hex::encode(&provider_identity);
                let identity_changed = stored.filesystem_identity != filesystem_identity
                    || stored.scope_root_identity != current_filesystem_identity
                    || stored.provider_identity != provider_identity
                    || stored.provider_id_text.as_deref()
                        != Some(current_provider_identity.as_str());
                if identity_changed && !replace_verified_stale_owner {
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
                if !replace_verified_stale_owner {
                    return Err(Error::DaemonUnavailable(
                        "persisted workspace daemon owner exists without verified stale process identity"
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
                    None => stored.observer_owner_token.is_none(),
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
            policy_generation: 1,
            filesystem_identity,
            provider_identity,
        };
        let stored_fingerprint: String = db.conn.query_row(
            "SELECT policy_fingerprint FROM changed_path_scopes WHERE scope_id=?1 AND epoch=?2",
            params![scope_id.to_text(), i64::try_from(epoch).unwrap_or(i64::MAX)],
            |row| row.get(0),
        )?;
        expected.policy_fingerprint = decode_fingerprint(&stored_fingerprint)?;
        let git_environment = std::env::vars_os().collect::<Vec<(OsString, OsString)>>();
        let mut metrics = PolicyDependencyMetrics::default();
        let mut policy = compile_policy(
            &db.conn,
            &expected,
            &PolicyCompileContext {
                workspace_root: &db.workspace_root,
                db_dir: &db.db_dir,
                recording: &db.config.recording,
                case_sensitive: true,
                git_environment: &git_environment,
            },
            &mut metrics,
        )?;
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
        let writer = SegmentWriter::acquire(
            &db.sqlite_path,
            &segment_directory,
            scope_id,
            epoch,
            owner,
            &hex::encode(&expected.provider_identity),
            resume_cursor.clone().unwrap_or_default(),
            Duration::from_secs(30),
        )?;
        let observer = PlatformObserver::start(
            db.workspace_root(),
            writer,
            expected.provider_identity.clone(),
            fence_nonce.to_vec(),
            policy.dependency_files(),
            resume_cursor.take(),
        )?;
        let compile_start = observer.begin_observation(&expected)?;
        let mut verified_metrics = PolicyDependencyMetrics::default();
        let verified_policy = compile_policy(
            &db.conn,
            &expected,
            &PolicyCompileContext {
                workspace_root: &db.workspace_root,
                db_dir: &db.db_dir,
                recording: &db.config.recording,
                case_sensitive: true,
                git_environment: &git_environment,
            },
            &mut verified_metrics,
        )?;
        let compile_end = observer.end_fence(&expected, &compile_start)?;
        let lease = observer.lease()?;
        observer.drain_through(
            &expected,
            &lease.root_identity,
            &compile_start,
            &compile_end,
            &mut |_event| Ok(()),
        )?;
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
            expected,
            policy,
            observer,
            last_cut: None,
        })
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
        db.changed_path_ledger().recover_scope(&self.expected)?;
        reconcile_full(
            db,
            &db.changed_path_ledger(),
            &self.observer,
            &self.expected,
            &self.policy,
            reason,
        )?;
        self.fence(db)
    }

    fn fence(&mut self, db: &Trail) -> Result<WorkspaceDaemonProof> {
        let start = self.observer.begin_observation(&self.expected)?;
        let fence = self.observer.end_fence(&self.expected, &start)?;
        let lease = self.observer.lease().map_err(|error| {
            Error::DaemonUnavailable(format!(
                "changed-path observer health no longer authorizes the ready proof: {error}"
            ))
        })?;
        self.observer.drain_through(
            &self.expected,
            &lease.root_identity,
            &start,
            &fence,
            &mut |_event| Ok(()),
        )?;
        let folded = db.conn.query_row(
            "SELECT folded_offset FROM changed_path_scopes WHERE scope_id=?1 AND epoch=?2",
            params![
                self.expected.scope_id.to_text(),
                i64::try_from(self.expected.epoch).unwrap_or(i64::MAX)
            ],
            |row| row.get::<_, i64>(0),
        )?;
        let cut = EvidenceCut {
            source: EvidenceSource::Observer,
            sequence: fence.sequence,
            durable_offset: fence.durable_offset,
            folded_offset: u64::try_from(folded)
                .map_err(|_| Error::Corrupt("negative daemon folded offset".into()))?,
        };
        self.last_cut = Some(cut.clone());
        Ok(WorkspaceDaemonProof {
            scope_id: self.expected.scope_id.to_text(),
            epoch: self.expected.epoch,
            cut,
        })
    }

    fn current_proof(&self) -> Result<WorkspaceDaemonProof> {
        let lease = self.observer.lease().map_err(|error| {
            Error::DaemonUnavailable(format!(
                "changed-path observer health no longer authorizes the ready proof: {error}"
            ))
        })?;
        if lease.root_identity != self.expected.filesystem_identity
            || lease.provider_identity != self.expected.provider_identity
            || lease.policy_dependencies != self.policy.dependency_files()
            || !self.policy.authorizes_reconciliation(&self.expected)
        {
            return Err(Error::DaemonUnavailable(
                "changed-path observer health no longer authorizes the ready proof".into(),
            ));
        }
        Ok(WorkspaceDaemonProof {
            scope_id: self.expected.scope_id.to_text(),
            epoch: self.expected.epoch,
            cut: self.last_cut.clone().ok_or_else(|| {
                Error::DaemonUnavailable("workspace daemon has no authenticated ready cut".into())
            })?,
        })
    }
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
            return Ok(Self::MacOs(
                super::observer::macos::MacOsFseventsObserver::start(
                    root,
                    Box::new(durability),
                    resume,
                    dependencies,
                )?,
            ));
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
}

fn workspace_scope_id(db: &Trail) -> ScopeId {
    let mut digest = Sha256::new();
    digest.update(b"trail-workspace-changed-path-scope-v1\0");
    digest.update(db.config.workspace.id.0.as_bytes());
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
                    owner.error_state,owner.error_at
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
