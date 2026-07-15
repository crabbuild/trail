use std::ffi::OsString;
use std::fs;
use std::fs::OpenOptions;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::Path;
use std::time::Duration;

use getrandom::getrandom;
use rusqlite::{params, OptionalExtension};
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
                let identity_changed = stored.filesystem_identity != filesystem_identity
                    || stored.provider_identity != provider_identity;
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
                let tx = db.conn.unchecked_transaction()?;
                tx.execute(
                    "UPDATE changed_path_observer_owners
                     SET lease_state='revoked', error_state='daemon_owner_replaced',
                         error_at=strftime('%s','now'), updated_at=strftime('%s','now')
                     WHERE scope_id=?1",
                    [scope_id.to_text()],
                )?;
                let changed = tx.execute(
                    "UPDATE changed_path_scopes
                     SET epoch=?1,
                         ref_name=?2, ref_generation=?3, change_id=?4, baseline_root_id=?5,
                         scope_root_identity=?6, filesystem_identity=?6,
                         provider_id=?7, provider_identity=?7,
                         durable_cursor=?8, linearizable_fence=?9, rename_pairing=?10,
                         overflow_scope=?11, filesystem_supported=?12,
                         clean_proof_allowed=?13, power_loss_durability=?14,
                         trust_state='untrusted_gap', trust_reason=?15,
                         observer_owner_token=NULL, provider_cursor=NULL, provider_fence=NULL,
                         observer_heartbeat_at=NULL,
                         durable_offset=0, folded_offset=0,
                         continuity_generation=continuity_generation+1,
                         updated_at=strftime('%s','now')
                     WHERE scope_id=?16 AND epoch=?17
                       AND filesystem_identity=?18 AND provider_identity=?19",
                    params![
                        i64::try_from(next)
                            .map_err(|_| Error::InvalidInput("epoch overflow".into()))?,
                        &baseline.ref_name,
                        i64::try_from(baseline.ref_generation)
                            .map_err(|_| Error::InvalidInput("ref generation overflow".into()))?,
                        &baseline.change_id.0,
                        &baseline.root_id.0,
                        hex::encode(&filesystem_identity),
                        hex::encode(&provider_identity),
                        if capabilities.durable_cursor {
                            1_i64
                        } else {
                            0_i64
                        },
                        if capabilities.linearizable_fence {
                            1_i64
                        } else {
                            0_i64
                        },
                        if capabilities.rename_pairing {
                            1_i64
                        } else {
                            0_i64
                        },
                        if capabilities.overflow_scope {
                            1_i64
                        } else {
                            0_i64
                        },
                        if capabilities.filesystem_supported {
                            1_i64
                        } else {
                            0_i64
                        },
                        if capabilities.clean_proof_allowed {
                            1_i64
                        } else {
                            0_i64
                        },
                        if capabilities.power_loss_durability {
                            1_i64
                        } else {
                            0_i64
                        },
                        if identity_changed {
                            "daemon_identity_transition"
                        } else {
                            "daemon_owner_restarted"
                        },
                        scope_id.to_text(),
                        i64::try_from(old_epoch)
                            .map_err(|_| Error::InvalidInput("epoch overflow".into()))?,
                        hex::encode(&stored.filesystem_identity),
                        hex::encode(&stored.provider_identity),
                    ],
                )?;
                if changed != 1 {
                    return Err(Error::ChangeLedgerReconcileRequired {
                        scope: scope_id.to_text(),
                        state: "stale_baseline".into(),
                        reason: "daemon baseline transition lost exact scope authority".into(),
                        command: "trail status".into(),
                    });
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
    epoch: u64,
    ref_name: String,
    ref_generation: u64,
    change_id: String,
    baseline_root: String,
    policy_fingerprint: [u8; 32],
    policy_generation: u64,
    filesystem_identity: Vec<u8>,
    provider_identity: Vec<u8>,
    provider_cursor: Option<Vec<u8>>,
}

fn load_existing_scope(db: &Trail, scope_id: ScopeId) -> Result<Option<ExistingScope>> {
    let row = db
        .conn
        .query_row(
            "SELECT epoch,ref_name,ref_generation,change_id,baseline_root_id,
                    policy_fingerprint,policy_dependency_generation,
                    filesystem_identity,provider_identity,provider_cursor
             FROM changed_path_scopes WHERE scope_id=?1",
            [scope_id.to_text()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, String>(8)?,
                    row.get::<_, Option<Vec<u8>>>(9)?,
                ))
            },
        )
        .optional()?;
    row.map(
        |(
            epoch,
            ref_name,
            ref_generation,
            change_id,
            baseline_root,
            policy_fingerprint,
            policy_generation,
            filesystem_identity,
            provider_identity,
            provider_cursor,
        )| {
            Ok(ExistingScope {
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
                provider_identity: hex::decode(provider_identity)
                    .map_err(|_| Error::Corrupt("invalid changed-path provider identity".into()))?,
                provider_cursor,
            })
        },
    )
    .transpose()
}
