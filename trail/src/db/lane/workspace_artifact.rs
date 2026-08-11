use super::*;
use crate::ids::{
    ArtifactAttemptId, ArtifactAttestationId, ArtifactBlobId, ArtifactChunkId, ArtifactChunkListId,
    ArtifactDesiredKeyV2, ArtifactEnvelopeId, ArtifactFileId, ArtifactQuarantineId, ArtifactTreeId,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};

const MAX_RESOLUTION_INPUTS: usize = 16_384;
const MAX_RESOLUTION_ARGV: usize = 1_024;
const MAX_RESOLUTION_AUTHORITIES: usize = 256;
const MAX_RESOLUTION_CREDENTIAL_HANDLES: usize = 64;
const MAX_RESOLUTION_ENVIRONMENT_NAMES: usize = 256;
const MAX_RESOLUTION_VALIDATIONS: usize = 256;
const MAX_RESOLUTION_PREDECESSORS: usize = 16_384;
const MAX_RESOLUTION_TEXT_BYTES: usize = 4 * 1024;
const MAX_RESOLUTION_TIMEOUT_MS: u64 = 60 * 60 * 1_000;
const MAX_RESOLUTION_CAPTURE_BYTES: u64 = 16 * 1024 * 1024;
const MAX_RESOLUTION_CANDIDATE_BYTES: u64 = 1024 * 1024 * 1024;
const MAX_RESOLUTION_CANDIDATE_ENTRIES: u64 = 1_000_000;
const MAX_RESOLUTION_CHILD_PROCESSES: u32 = 256;
const ARTIFACT_DESIRED_KEY_MATERIAL_VERSION: u16 = 2;
const ARTIFACT_WHOLE_BLOB_MAX_BYTES: usize = 1024 * 1024;
const ARTIFACT_CHUNK_MIN_BYTES: usize = 256 * 1024;
const ARTIFACT_CHUNK_AVERAGE_BYTES: usize = 1024 * 1024;
const ARTIFACT_CHUNK_MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_ARTIFACT_TREE_ENTRIES: u64 = 1_000_000;
const MAX_ARTIFACT_TREE_LOGICAL_BYTES: u64 = 1024 * 1024 * 1024 * 1024;
const MAX_ARTIFACT_TREE_DEPTH: usize = 256;
const MAX_PUBLIC_ARTIFACT_REPORT_ITEMS: usize = 10_000;
const MAX_PUBLIC_ARTIFACT_OBJECT_REFERENCES: usize = 10_000_000;
const HOST_WORKSPACE_LAYER_SEAL_VALIDATOR: &str = "trail.host/workspace-layer-sealer@1";
pub(crate) const HOST_WORKSPACE_LAYER_STRUCTURAL_SEAL: &str =
    "trail.host.workspace-layer.structural-seal/v1";
const HOST_WORKSPACE_LAYER_POLICY_SEAL: &str = "trail.host.workspace-layer.policy-seal/v1";

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArtifactResolutionAttemptFence {
    pub(crate) attempt_id: ArtifactAttemptId,
    pub(crate) owner_generation: u64,
    pub(crate) owner_pid: u32,
    pub(crate) owner_start_token: String,
}

pub(crate) struct ArtifactResolutionAttemptFailure<'a> {
    pub(crate) code: &'a str,
    pub(crate) message: &'a str,
    pub(crate) contacted_authorities: Vec<String>,
    pub(crate) stdout: &'a [u8],
    pub(crate) stderr: &'a [u8],
    pub(crate) stdout_original_bytes: Option<u64>,
    pub(crate) stderr_original_bytes: Option<u64>,
    pub(crate) redactions: &'a [Vec<u8>],
    pub(crate) cancelled: bool,
}

pub(crate) struct ArtifactResolutionExecutorFailure {
    pub(crate) code: String,
    pub(crate) message: String,
    pub(crate) contacted_authorities: Vec<String>,
    pub(crate) stdout: Vec<u8>,
    pub(crate) stderr: Vec<u8>,
    pub(crate) stdout_original_bytes: u64,
    pub(crate) stderr_original_bytes: u64,
    pub(crate) redactions: Vec<Vec<u8>>,
    pub(crate) cancelled: bool,
}

pub(crate) type ArtifactResolutionExecutorResult =
    std::result::Result<ArtifactResolutionCandidateV1, Box<ArtifactResolutionExecutorFailure>>;

impl ArtifactResolutionExecutorFailure {
    pub(crate) fn from_error(code: &str, error: impl std::fmt::Display) -> Box<Self> {
        Box::new(Self {
            code: code.to_string(),
            message: error.to_string(),
            contacted_authorities: Vec::new(),
            stdout: Vec::new(),
            stderr: Vec::new(),
            stdout_original_bytes: 0,
            stderr_original_bytes: 0,
            redactions: Vec::new(),
            cancelled: false,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArtifactFlatEntry {
    pub(crate) kind: &'static str,
    pub(crate) mode: u32,
    pub(crate) size_bytes: u64,
    pub(crate) content_hash: Option<String>,
    pub(crate) symlink_target: Option<String>,
}

/// One path resolved directly from an immutable artifact manifest. Directory
/// and file identities remain available so callers can continue traversal or
/// read/copy only the selected file without projecting the complete tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ArtifactLazyEntry {
    Directory {
        node_id: ArtifactTreeId,
    },
    File {
        node_id: ArtifactFileId,
        mode: u32,
        size_bytes: u64,
    },
    Symlink {
        target: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArtifactMaterializationReport {
    pub(crate) materialization_id: String,
    pub(crate) tree_root_id: ArtifactTreeId,
    pub(crate) backend_compatibility: String,
    pub(crate) storage_path: PathBuf,
    pub(crate) logical_bytes: u64,
    pub(crate) physical_bytes: u64,
    pub(crate) entry_count: u64,
    pub(crate) reused: bool,
}

impl Trail {
    /// Publish or deliberately refresh one resolver-produced snapshot.
    ///
    /// The executor remains a separate capability boundary: this operation
    /// owns pin validation, attempt fencing, bounded/redacted evidence,
    /// content-addressed snapshot publication, and reuse. A current snapshot
    /// is reused without inspecting candidate bytes unless `refresh` is true;
    /// wall-clock time never advances dependency selection.
    pub fn resolve_artifact_component(
        &self,
        request: ArtifactResolutionRequestV1,
        refresh: bool,
    ) -> Result<ArtifactResolutionComponentReportV1> {
        let ArtifactResolutionRequestV1 { plan, candidate } = request;
        self.resolve_artifact_component_with_executor(plan, refresh, |_, _| Ok(candidate))
    }

    /// Run one host-owned resolver inside the same durable attempt that later
    /// validates and publishes its candidate. Process launch and candidate
    /// production failures therefore retain the same fenced recovery evidence
    /// as malformed or rejected candidates.
    pub(crate) fn resolve_artifact_component_with_executor<F>(
        &self,
        mut plan: ArtifactResolutionPlanV1,
        refresh: bool,
        executor: F,
    ) -> Result<ArtifactResolutionComponentReportV1>
    where
        F: FnOnce(
            &ArtifactResolutionPlanV1,
            &ArtifactResolutionAttemptFence,
        ) -> ArtifactResolutionExecutorResult,
    {
        normalize_artifact_resolution_plan(&mut plan)?;
        self.validate_artifact_resolution_plan_pins(&plan)?;
        if !plan.credential_handles.is_empty() {
            return Err(Error::InvalidInput(
                "artifact resolution declares credential access; secret-influenced resolver output must remain lane-private and cannot enter shared CAS"
                    .into(),
            ));
        }

        if let Some((snapshot_id, snapshot)) =
            self.artifact_resolution_snapshot_for_proposal(&plan.proposal_key)?
            && !refresh
        {
            if snapshot.source_root != plan.source_root
                || snapshot.component_id != plan.component_id
                || snapshot.adapter_identity != plan.adapter_identity
                || snapshot.snapshot_format != plan.snapshot_format
                || snapshot.resolver_executable_identity != plan.executable_identity
                || snapshot.policy_identity != plan.policy_identity
            {
                return Err(Error::InvalidInput(format!(
                    "artifact proposal key `{}` resolves to stale or incompatible snapshot {}; change the proposal key or request an explicit refresh",
                    plan.proposal_key, snapshot_id
                )));
            }
            return Ok(ArtifactResolutionComponentReportV1 {
                component_id: plan.component_id,
                proposal_key: plan.proposal_key,
                source_root: plan.source_root,
                snapshot_id,
                snapshot,
                decision: ArtifactResolutionDecisionV1::Reused,
                refresh_requested: false,
                attempt: None,
            });
        }

        let (fence, _) = self.begin_artifact_resolution_attempt(plan.clone())?;
        let candidate = match executor(&plan, &fence) {
            Ok(candidate) => candidate,
            Err(failure) => {
                let attempt = self.finish_artifact_resolution_attempt_failure(
                    &fence,
                    ArtifactResolutionAttemptFailure {
                        code: &failure.code,
                        message: &failure.message,
                        contacted_authorities: failure.contacted_authorities,
                        stdout: &failure.stdout,
                        stderr: &failure.stderr,
                        stdout_original_bytes: Some(failure.stdout_original_bytes),
                        stderr_original_bytes: Some(failure.stderr_original_bytes),
                        redactions: &failure.redactions,
                        cancelled: failure.cancelled,
                    },
                )?;
                return Err(Error::InvalidInput(format!(
                    "artifact resolution attempt `{}` failed during resolver execution: {}",
                    attempt.attempt_id, failure.message
                )));
            }
        };
        let ArtifactResolutionCandidateV1 {
            snapshot_bytes,
            resolved_identities,
            checksums,
            contacted_authorities,
            stdout,
            stderr,
            redactions,
        } = candidate;
        if redactions.iter().any(|secret| !secret.is_empty()) {
            let message = "resolver consumed secret material; its output is private, non-promotable, and cannot enter shared CAS";
            let attempt = self.finish_artifact_resolution_attempt_failure(
                &fence,
                ArtifactResolutionAttemptFailure {
                    code: "secret_tainted_output_private_only",
                    message,
                    contacted_authorities,
                    stdout: &stdout,
                    stderr: &stderr,
                    stdout_original_bytes: None,
                    stderr_original_bytes: None,
                    redactions: &redactions,
                    cancelled: false,
                },
            )?;
            return Err(Error::InvalidInput(format!(
                "artifact resolution attempt `{}` failed: {message}",
                attempt.attempt_id
            )));
        }
        if snapshot_bytes.is_empty() {
            let message = "resolver produced an empty or malformed snapshot candidate";
            let attempt = self.finish_artifact_resolution_attempt_failure(
                &fence,
                ArtifactResolutionAttemptFailure {
                    code: "malformed_resolution_candidate",
                    message,
                    contacted_authorities: Vec::new(),
                    stdout: &stdout,
                    stderr: &stderr,
                    stdout_original_bytes: None,
                    stderr_original_bytes: None,
                    redactions: &redactions,
                    cancelled: false,
                },
            )?;
            return Err(Error::InvalidInput(format!(
                "artifact resolution attempt `{}` failed: {message}",
                attempt.attempt_id
            )));
        }
        if stdout.len() as u64 > plan.limits.stdout_bytes
            || stderr.len() as u64 > plan.limits.stderr_bytes
        {
            let message = "resolver output exceeded its declared capture limit";
            let attempt = self.finish_artifact_resolution_attempt_failure(
                &fence,
                ArtifactResolutionAttemptFailure {
                    code: "captured_output_limit_exceeded",
                    message,
                    contacted_authorities: Vec::new(),
                    stdout: &stdout,
                    stderr: &stderr,
                    stdout_original_bytes: None,
                    stderr_original_bytes: None,
                    redactions: &redactions,
                    cancelled: false,
                },
            )?;
            return Err(Error::InvalidInput(format!(
                "artifact resolution attempt `{}` failed: {message}",
                attempt.attempt_id
            )));
        }
        let publication = self.put_artifact_resolution_snapshot(
            plan.clone(),
            snapshot_bytes,
            resolved_identities,
            checksums,
            contacted_authorities.clone(),
            refresh,
        );
        let (snapshot_id, snapshot) = match publication {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let message = error.to_string();
                if let Err(finish_error) = self.finish_artifact_resolution_attempt_failure(
                    &fence,
                    ArtifactResolutionAttemptFailure {
                        code: "resolution_candidate_rejected",
                        message: &message,
                        contacted_authorities: Vec::new(),
                        stdout: &stdout,
                        stderr: &stderr,
                        stdout_original_bytes: None,
                        stderr_original_bytes: None,
                        redactions: &redactions,
                        cancelled: false,
                    },
                ) {
                    return Err(Error::Corrupt(format!(
                        "artifact resolution candidate was rejected ({message}) and attempt `{}` could not record failure: {finish_error}",
                        fence.attempt_id
                    )));
                }
                return Err(error);
            }
        };
        let attempt = self.finish_artifact_resolution_attempt_success(
            &fence,
            &snapshot_id,
            contacted_authorities,
            &stdout,
            &stderr,
            &redactions,
        )?;
        if attempt.status != ArtifactResolutionAttemptStatusV1::Succeeded {
            return Err(Error::InvalidInput(format!(
                "artifact resolution attempt `{}` failed while recording bounded output",
                attempt.attempt_id
            )));
        }
        Ok(ArtifactResolutionComponentReportV1 {
            component_id: plan.component_id,
            proposal_key: plan.proposal_key,
            source_root: plan.source_root,
            snapshot_id,
            snapshot,
            decision: if refresh {
                ArtifactResolutionDecisionV1::Refreshed
            } else {
                ArtifactResolutionDecisionV1::Resolved
            },
            refresh_requested: refresh,
            attempt: Some(attempt),
        })
    }

    /// Resolve a deterministic set of component requests for one pinned root.
    pub fn resolve_all_artifact_components(
        &self,
        mut requests: Vec<ArtifactResolutionRequestV1>,
        refresh: bool,
    ) -> Result<ArtifactResolutionBatchReportV1> {
        if requests.is_empty() {
            return Err(Error::InvalidInput(
                "artifact resolve-all requires at least one component request".into(),
            ));
        }
        requests.sort_by(|left, right| {
            (&left.plan.component_id, &left.plan.proposal_key)
                .cmp(&(&right.plan.component_id, &right.plan.proposal_key))
        });
        if requests.windows(2).any(|pair| {
            pair[0].plan.component_id == pair[1].plan.component_id
                || pair[0].plan.proposal_key == pair[1].plan.proposal_key
        }) {
            return Err(Error::InvalidInput(
                "artifact resolve-all contains a duplicate component or proposal key".into(),
            ));
        }
        let source_root = requests[0].plan.source_root.clone();
        if requests
            .iter()
            .any(|request| request.plan.source_root != source_root)
        {
            return Err(Error::InvalidInput(
                "artifact resolve-all requests must pin one source root".into(),
            ));
        }
        let components = requests
            .into_iter()
            .map(|request| self.resolve_artifact_component(request, refresh))
            .collect::<Result<Vec<_>>>()?;
        Ok(ArtifactResolutionBatchReportV1 {
            source_root,
            refresh_requested: refresh,
            components,
        })
    }

    pub(crate) fn begin_artifact_resolution_attempt(
        &self,
        mut plan: ArtifactResolutionPlanV1,
    ) -> Result<(
        ArtifactResolutionAttemptFence,
        ArtifactResolutionAttemptReportV1,
    )> {
        let _lock = self.acquire_write_lock()?;
        normalize_artifact_resolution_plan(&mut plan)?;
        self.validate_artifact_resolution_plan_pins(&plan)?;
        self.recover_artifact_resolution_attempts_under_write_lock()?;

        if let Some((attempt_id, owner_pid)) = self
            .conn
            .query_row(
                "SELECT attempt_id, owner_pid FROM artifact_resolution_attempts
                 WHERE proposal_key=?1 AND status='running'",
                params![plan.proposal_key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )
            .optional()?
        {
            return Err(Error::InvalidInput(format!(
                "artifact proposal `{}` is already resolving in attempt `{attempt_id}` owned by process {owner_pid}",
                plan.proposal_key
            )));
        }

        let plan_object_id = self.put_object(
            ARTIFACT_RESOLUTION_PLAN_KIND,
            ARTIFACT_RESOLUTION_PLAN_VERSION,
            &plan,
        )?;
        let owner_generation = self.conn.query_row(
            "SELECT COALESCE(MAX(owner_generation), 0) + 1
             FROM artifact_resolution_attempts WHERE proposal_key=?1",
            params![plan.proposal_key],
            |row| row.get::<_, i64>(0),
        )?;
        let owner_generation = u64::try_from(owner_generation).map_err(|_| {
            Error::Corrupt(
                "artifact resolution owner generation is outside the supported range".into(),
            )
        })?;
        let owner_pid = std::process::id();
        let owner_start_token = current_process_start_token();
        let attempt_id = ArtifactAttemptId::new(
            format!(
                "resolution\0{}\0{}\0{owner_generation}\0{owner_pid}\0{owner_start_token}\0{}",
                plan.proposal_key,
                plan.source_root,
                now_nanos()
            )
            .as_bytes(),
        );
        let authority_evidence = ArtifactResolutionAuthorityEvidenceV1 {
            allowed_authorities: plan.allowed_authorities.clone(),
            contacted_authorities: Vec::new(),
            credential_handles: plan.credential_handles.clone(),
            credential_values_redacted: true,
        };
        let authority_evidence_json = serde_json::to_vec(&authority_evidence)?;
        self.conn.execute(
            "INSERT INTO artifact_resolution_attempts(
                attempt_id, proposal_key, source_root, plan_object_id,
                owner_generation, owner_pid, owner_start_token, status,
                cancel_requested, authority_evidence_json, stdout_object_id,
                stderr_object_id, snapshot_id, failure_receipt_object_id,
                failure_code, failure_message, started_at, heartbeat_at, finished_at
             ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, 'running', 0, ?8,
                      NULL, NULL, NULL, NULL, NULL, NULL, ?9, ?9, NULL)",
            params![
                attempt_id.0,
                plan.proposal_key,
                plan.source_root.0,
                plan_object_id.0,
                i64::try_from(owner_generation).map_err(|_| Error::InvalidInput(
                    "artifact resolution owner generation exceeds SQLite range".into()
                ))?,
                i64::from(owner_pid),
                owner_start_token,
                authority_evidence_json,
                now_ts(),
            ],
        )?;
        let fence = ArtifactResolutionAttemptFence {
            attempt_id: attempt_id.clone(),
            owner_generation,
            owner_pid,
            owner_start_token,
        };
        let report = self.artifact_resolution_attempt(&attempt_id)?;
        Ok((fence, report))
    }

    pub(crate) fn heartbeat_artifact_resolution_attempt(
        &self,
        fence: &ArtifactResolutionAttemptFence,
    ) -> Result<bool> {
        let _lock = self.acquire_write_lock()?;
        let updated = self.conn.execute(
            "UPDATE artifact_resolution_attempts SET heartbeat_at=?1
             WHERE attempt_id=?2 AND owner_generation=?3 AND owner_pid=?4
               AND owner_start_token=?5 AND status='running' AND cancel_requested=0",
            params![
                now_ts(),
                fence.attempt_id.0,
                i64::try_from(fence.owner_generation).map_err(|_| Error::InvalidInput(
                    "artifact resolution owner generation exceeds SQLite range".into()
                ))?,
                i64::from(fence.owner_pid),
                fence.owner_start_token,
            ],
        )?;
        Ok(updated == 1)
    }

    pub(crate) fn cancel_artifact_resolution_attempt(
        &self,
        attempt_id: &ArtifactAttemptId,
    ) -> Result<ArtifactResolutionAttemptReportV1> {
        let _lock = self.acquire_write_lock()?;
        let updated = self.conn.execute(
            "UPDATE artifact_resolution_attempts
             SET cancel_requested=1, heartbeat_at=?1
             WHERE attempt_id=?2 AND status='running'",
            params![now_ts(), attempt_id.0],
        )?;
        if updated == 0 {
            let existing = self.artifact_resolution_attempt(attempt_id)?;
            if existing.status == ArtifactResolutionAttemptStatusV1::Running {
                return Err(Error::Corrupt(format!(
                    "artifact resolution attempt `{attempt_id}` could not be cancelled"
                )));
            }
            return Ok(existing);
        }
        self.artifact_resolution_attempt(attempt_id)
    }

    pub(crate) fn finish_artifact_resolution_attempt_success(
        &self,
        fence: &ArtifactResolutionAttemptFence,
        snapshot_id: &ObjectId,
        contacted_authorities: Vec<String>,
        stdout: &[u8],
        stderr: &[u8],
        redactions: &[Vec<u8>],
    ) -> Result<ArtifactResolutionAttemptReportV1> {
        let _lock = self.acquire_write_lock()?;
        let plan = self.artifact_resolution_plan_for_fence(fence)?;
        self.validate_artifact_resolution_plan_pins(&plan)?;
        let snapshot = self.get_object::<ArtifactResolutionSnapshotV1>(
            ARTIFACT_RESOLUTION_SNAPSHOT_KIND,
            snapshot_id,
        )?;
        validate_artifact_resolution_snapshot(&snapshot)?;
        if snapshot.proposal_key != plan.proposal_key || snapshot.source_root != plan.source_root {
            return Err(Error::InvalidInput(
                "artifact resolution snapshot does not match the fenced proposal/source pins"
                    .into(),
            ));
        }
        let authority_evidence =
            normalized_resolution_authority_evidence(&plan, contacted_authorities)?;
        let (stdout_object_id, stdout_truncated) =
            self.put_artifact_resolution_capture(stdout, plan.limits.stdout_bytes, redactions)?;
        let (stderr_object_id, stderr_truncated) =
            self.put_artifact_resolution_capture(stderr, plan.limits.stderr_bytes, redactions)?;
        if stdout_truncated || stderr_truncated {
            let secret_tainted = resolution_is_secret_tainted(&plan, redactions);
            return self.finish_artifact_resolution_attempt_failure_under_write_lock(
                fence,
                "captured_output_limit_exceeded",
                "resolver output exceeded its declared capture limit",
                authority_evidence,
                stdout_object_id,
                stderr_object_id,
                secret_tainted,
                ArtifactResolutionAttemptStatusV1::Failed,
            );
        }
        let updated = self.conn.execute(
            "UPDATE artifact_resolution_attempts
             SET status='succeeded', authority_evidence_json=?1,
                 stdout_object_id=?2, stderr_object_id=?3, snapshot_id=?4,
                 heartbeat_at=?5, finished_at=?5
             WHERE attempt_id=?6 AND owner_generation=?7 AND owner_pid=?8
               AND owner_start_token=?9 AND status='running' AND cancel_requested=0",
            params![
                serde_json::to_vec(&authority_evidence)?,
                stdout_object_id.as_ref().map(|id| id.0.as_str()),
                stderr_object_id.as_ref().map(|id| id.0.as_str()),
                snapshot_id.0,
                now_ts(),
                fence.attempt_id.0,
                i64::try_from(fence.owner_generation).map_err(|_| Error::InvalidInput(
                    "artifact resolution owner generation exceeds SQLite range".into()
                ))?,
                i64::from(fence.owner_pid),
                fence.owner_start_token,
            ],
        )?;
        if updated != 1 {
            return Err(Error::InvalidInput(format!(
                "artifact resolution attempt `{}` lost its owner fence or was cancelled",
                fence.attempt_id
            )));
        }
        self.artifact_resolution_attempt(&fence.attempt_id)
    }

    pub(crate) fn finish_artifact_resolution_attempt_failure(
        &self,
        fence: &ArtifactResolutionAttemptFence,
        failure: ArtifactResolutionAttemptFailure<'_>,
    ) -> Result<ArtifactResolutionAttemptReportV1> {
        let _lock = self.acquire_write_lock()?;
        let plan = self.artifact_resolution_plan_for_fence(fence)?;
        validate_resolution_text(failure.code, "failure code")?;
        validate_resolution_text(failure.message, "failure message")?;
        let authority_evidence =
            normalized_resolution_authority_evidence(&plan, failure.contacted_authorities)?;
        let (stdout_object_id, _) = self.put_artifact_resolution_capture_observed(
            failure.stdout,
            failure
                .stdout_original_bytes
                .unwrap_or_else(|| u64::try_from(failure.stdout.len()).unwrap_or(u64::MAX)),
            plan.limits.stdout_bytes,
            failure.redactions,
        )?;
        let (stderr_object_id, _) = self.put_artifact_resolution_capture_observed(
            failure.stderr,
            failure
                .stderr_original_bytes
                .unwrap_or_else(|| u64::try_from(failure.stderr.len()).unwrap_or(u64::MAX)),
            plan.limits.stderr_bytes,
            failure.redactions,
        )?;
        let redacted_message = String::from_utf8_lossy(&redact_resolution_bytes(
            failure.message.as_bytes(),
            failure.redactions,
        ))
        .into_owned();
        let secret_tainted = resolution_is_secret_tainted(&plan, failure.redactions);
        self.finish_artifact_resolution_attempt_failure_under_write_lock(
            fence,
            failure.code,
            &redacted_message,
            authority_evidence,
            stdout_object_id,
            stderr_object_id,
            secret_tainted,
            if failure.cancelled {
                ArtifactResolutionAttemptStatusV1::Cancelled
            } else {
                ArtifactResolutionAttemptStatusV1::Failed
            },
        )
    }

    pub(crate) fn artifact_resolution_attempt(
        &self,
        attempt_id: &ArtifactAttemptId,
    ) -> Result<ArtifactResolutionAttemptReportV1> {
        let row = self.conn.query_row(
            "SELECT proposal_key, source_root, plan_object_id, owner_generation,
                    owner_pid, status, cancel_requested, authority_evidence_json,
                    stdout_object_id, stderr_object_id, snapshot_id,
                    failure_receipt_object_id, failure_code, failure_message,
                    started_at, heartbeat_at, finished_at
             FROM artifact_resolution_attempts WHERE attempt_id=?1",
            params![attempt_id.0],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, bool>(6)?,
                    row.get::<_, Vec<u8>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                    row.get::<_, Option<String>>(9)?,
                    row.get::<_, Option<String>>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, Option<String>>(12)?,
                    row.get::<_, Option<String>>(13)?,
                    row.get::<_, i64>(14)?,
                    row.get::<_, i64>(15)?,
                    row.get::<_, Option<i64>>(16)?,
                ))
            },
        )?;
        let plan: ArtifactResolutionPlanV1 =
            self.get_object(ARTIFACT_RESOLUTION_PLAN_KIND, &ObjectId(row.2.clone()))?;
        Ok(ArtifactResolutionAttemptReportV1 {
            attempt_id: attempt_id.clone(),
            proposal_key: row.0,
            source_root: ObjectId(row.1),
            plan_object_id: ObjectId(row.2),
            owner_generation: u64::try_from(row.3).map_err(|_| {
                Error::Corrupt("artifact resolution owner generation is invalid".into())
            })?,
            owner_pid: u32::try_from(row.4)
                .map_err(|_| Error::Corrupt("artifact resolution owner PID is invalid".into()))?,
            status: parse_artifact_resolution_attempt_status(&row.5)?,
            cancel_requested: row.6,
            authority_evidence: serde_json::from_slice(&row.7).map_err(|error| {
                Error::Corrupt(format!(
                    "invalid artifact resolution authority evidence: {error}"
                ))
            })?,
            stdout_object_id: row.8.map(ObjectId),
            stderr_object_id: row.9.map(ObjectId),
            snapshot_id: row.10.map(ObjectId),
            failure_receipt_object_id: row.11.map(ObjectId),
            failure_code: row.12,
            failure_message: row.13,
            started_at: row.14,
            heartbeat_at: row.15,
            finished_at: row.16,
            recovery_command: vec![
                "trail".into(),
                "env".into(),
                "resolve".into(),
                "component".into(),
                plan.component_id,
            ],
        })
    }

    pub(crate) fn artifact_resolution_attempts(
        &self,
    ) -> Result<Vec<ArtifactResolutionAttemptReportV1>> {
        let mut stmt = self.conn.prepare(
            "SELECT attempt_id FROM artifact_resolution_attempts
             ORDER BY started_at, attempt_id",
        )?;
        let ids = stmt
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| {
                ArtifactAttemptId::parse(id)
                    .map_err(Error::Corrupt)
                    .and_then(|id| self.artifact_resolution_attempt(&id))
            })
            .collect()
    }

    pub(crate) fn recover_artifact_resolution_attempts_under_write_lock(&self) -> Result<()> {
        let running = {
            let mut stmt = self.conn.prepare(
                "SELECT attempt_id, owner_generation, owner_pid, owner_start_token,
                        cancel_requested
                 FROM artifact_resolution_attempts WHERE status='running'
                 ORDER BY started_at, attempt_id",
            )?;
            stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, bool>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (attempt_id, owner_generation, owner_pid, owner_start_token, cancel_requested) in
            running
        {
            let Ok(owner_pid_u32) = u32::try_from(owner_pid) else {
                continue;
            };
            if process_start_token_match(owner_pid_u32, &owner_start_token)
                != ProcessIdentityMatch::DeadOrMismatch
            {
                continue;
            }
            let fence = ArtifactResolutionAttemptFence {
                attempt_id: ArtifactAttemptId::parse(attempt_id).map_err(Error::Corrupt)?,
                owner_generation: u64::try_from(owner_generation).map_err(|_| {
                    Error::Corrupt("artifact resolution owner generation is invalid".into())
                })?,
                owner_pid: owner_pid_u32,
                owner_start_token,
            };
            let plan = self.artifact_resolution_plan_for_fence(&fence)?;
            let evidence = normalized_resolution_authority_evidence(&plan, Vec::new())?;
            let secret_tainted = !plan.credential_handles.is_empty();
            self.finish_artifact_resolution_attempt_failure_under_write_lock(
                &fence,
                if cancel_requested {
                    "resolver_cancelled"
                } else {
                    "resolver_owner_lost"
                },
                if cancel_requested {
                    "resolver cancellation was recovered after its owner process exited"
                } else {
                    "resolver owner process exited before publishing a snapshot; retry the reported resolution command"
                },
                evidence,
                None,
                None,
                secret_tainted,
                if cancel_requested {
                    ArtifactResolutionAttemptStatusV1::Cancelled
                } else {
                    ArtifactResolutionAttemptStatusV1::Abandoned
                },
            )?;
        }
        Ok(())
    }

    /// Validate artifact CAS identities, edges, snapshots, envelopes, and
    /// durable attempt state without mutating repairable evidence.
    pub(crate) fn validate_artifact_cas_integrity(&self) -> Result<Vec<String>> {
        let mut errors = Vec::new();
        let objects = {
            let mut statement = self.conn.prepare(
                "SELECT a.artifact_id,a.kind,a.version,a.logical_bytes,a.object_id,
                        o.kind,o.version,o.codec,o.hash_alg,o.size_bytes,o.bytes
                 FROM artifact_objects a LEFT JOIN objects o ON o.object_id=a.object_id
                 ORDER BY a.artifact_id",
            )?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, Option<String>>(8)?,
                        row.get::<_, Option<i64>>(9)?,
                        row.get::<_, Option<Vec<u8>>>(10)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (
            artifact_id,
            kind,
            version,
            logical_bytes,
            object_id,
            object_kind,
            object_version,
            codec,
            hash_alg,
            size_bytes,
            bytes,
        ) in objects
        {
            let result = (|| -> Result<()> {
                let bytes = bytes.as_deref().ok_or_else(|| {
                    Error::Corrupt(format!("backing object {object_id} is missing"))
                })?;
                let encoded_version = u16::try_from(version).map_err(|_| {
                    Error::Corrupt(format!("artifact object version {version} is invalid"))
                })?;
                if object_kind.as_deref() != Some(kind.as_str())
                    || object_version != Some(version)
                    || codec.as_deref() != Some("cbor")
                    || hash_alg.as_deref() != Some("sha256")
                    || size_bytes != i64::try_from(bytes.len()).ok()
                    || ObjectId::for_bytes(&kind, encoded_version, bytes).0 != object_id
                {
                    return Err(Error::Corrupt(format!(
                        "backing object {object_id} metadata or content identity is invalid"
                    )));
                }
                self.validate_artifact_cas_object(
                    &artifact_id,
                    &kind,
                    version,
                    logical_bytes,
                    bytes,
                )
            })();
            if let Err(error) = result {
                errors.push(format!(
                    "artifact object {artifact_id} is corrupt: {error}; rebuild the owning environment component before reattaching it"
                ));
            }
        }

        let validation_receipts = {
            let mut statement = self
                .conn
                .prepare("SELECT object_id FROM objects WHERE kind=?1 ORDER BY object_id")?;
            statement
                .query_map(params![ARTIFACT_VALIDATION_RECEIPT_KIND], |row| {
                    row.get::<_, String>(0)
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for receipt_id in validation_receipts {
            if let Err(error) = self.artifact_validation_receipt(&ObjectId(receipt_id.clone())) {
                errors.push(format!(
                    "artifact validation receipt {receipt_id} is corrupt: {error}; rerun the exact validator before publishing or attaching the artifact"
                ));
            }
        }

        let snapshots = {
            let mut statement = self.conn.prepare(
                "SELECT snapshot_id,proposal_key,source_root,component_id,adapter_identity,
                        content_object_id,verification_state,state
                 FROM artifact_resolution_snapshots ORDER BY snapshot_id",
            )?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (
            snapshot_id,
            proposal_key,
            source_root,
            component_id,
            adapter_identity,
            content_object_id,
            verification_state,
            state,
        ) in snapshots
        {
            let result = (|| -> Result<()> {
                let snapshot_bytes = self.validated_raw_object_bytes(
                    ARTIFACT_RESOLUTION_SNAPSHOT_KIND,
                    &ObjectId(snapshot_id.clone()),
                    ARTIFACT_RESOLUTION_SNAPSHOT_VERSION,
                )?;
                let snapshot: ArtifactResolutionSnapshotV1 = from_cbor(&snapshot_bytes)?;
                validate_artifact_resolution_snapshot(&snapshot)?;
                if snapshot.proposal_key != proposal_key
                    || snapshot.source_root.0 != source_root
                    || snapshot.component_id != component_id
                    || snapshot.adapter_identity != adapter_identity
                    || snapshot.content_object_id.0 != content_object_id
                    || verification_state != "verified"
                    || !matches!(state.as_str(), "current" | "superseded")
                {
                    return Err(Error::Corrupt(
                        "snapshot database identity disagrees with its object".into(),
                    ));
                }
                let content_bytes = self.validated_raw_object_bytes(
                    ARTIFACT_RESOLUTION_CONTENT_KIND,
                    &snapshot.content_object_id,
                    ARTIFACT_RESOLUTION_SNAPSHOT_VERSION,
                )?;
                let content: ArtifactResolutionContentV1 = from_cbor(&content_bytes)?;
                if content.version != ARTIFACT_RESOLUTION_SNAPSHOT_VERSION
                    || content.content_sha256 != snapshot.content_sha256
                    || sha256_hex(&content.bytes) != snapshot.content_sha256
                {
                    return Err(Error::Corrupt(
                        "snapshot content failed identity verification".into(),
                    ));
                }
                Ok(())
            })();
            if let Err(error) = result {
                errors.push(format!(
                    "artifact resolution snapshot {snapshot_id} is corrupt: {error}; run explicit component resolution with refresh after restoring or reinitializing the workspace"
                ));
            }
        }

        let envelopes = {
            let mut statement = self.conn.prepare(
                "SELECT e.envelope_id,e.tree_root_id,e.object_id,a.object_id,
                        e.state,e.verification_state
                 FROM artifact_envelopes e
                 LEFT JOIN artifact_objects a ON a.artifact_id=e.envelope_id
                 ORDER BY e.envelope_id",
            )?;
            statement
                .query_map([], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        for (envelope_id, tree_id, object_id, artifact_object_id, state, verification) in envelopes
        {
            let result = (|| -> Result<()> {
                if artifact_object_id.as_deref() != Some(object_id.as_str()) {
                    return Err(Error::Corrupt(
                        "envelope object mapping is missing or cross-wired".into(),
                    ));
                }
                let envelope_id =
                    ArtifactEnvelopeId::parse(envelope_id.clone()).map_err(Error::Corrupt)?;
                let tree_id = ArtifactTreeId::parse(tree_id).map_err(Error::Corrupt)?;
                if state == "ready" && verification == "verified" {
                    self.verify_ready_artifact_envelope_under_write_lock(&envelope_id, &tree_id)?;
                    self.artifact_tree_flat_entries(&tree_id)?;
                }
                Ok(())
            })();
            if let Err(error) = result {
                errors.push(format!(
                    "artifact envelope {envelope_id} is corrupt: {error}; detach affected generations and rebuild the component before reuse"
                ));
            }
        }

        errors.extend(self.validate_artifact_attempt_integrity()?);
        Ok(errors)
    }

    fn validated_raw_object_bytes(
        &self,
        kind: &'static str,
        object_id: &ObjectId,
        version: u16,
    ) -> Result<Vec<u8>> {
        let row = self
            .conn
            .query_row(
                "SELECT kind,version,codec,hash_alg,size_bytes,bytes
                 FROM objects WHERE object_id=?1",
                params![object_id.0],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, Vec<u8>>(5)?,
                    ))
                },
            )
            .optional()?;
        let Some((stored_kind, stored_version, codec, hash_alg, size_bytes, bytes)) = row else {
            return Err(Error::ObjectNotFound {
                kind,
                id: object_id.0.clone(),
            });
        };
        if stored_kind != kind
            || stored_version != i64::from(version)
            || codec != "cbor"
            || hash_alg != "sha256"
            || i64::try_from(bytes.len()).ok() != Some(size_bytes)
            || ObjectId::for_bytes(kind, version, &bytes) != *object_id
        {
            return Err(Error::Corrupt(format!(
                "object {object_id} metadata or content identity is invalid"
            )));
        }
        Ok(bytes)
    }

    fn validate_artifact_cas_object(
        &self,
        artifact_id: &str,
        kind: &str,
        version: i64,
        logical_bytes: i64,
        bytes: &[u8],
    ) -> Result<()> {
        if logical_bytes < 0 {
            return Err(Error::Corrupt("logical byte count is negative".into()));
        }
        let expected_version = match kind {
            ARTIFACT_DIRECTORY_NODE_KIND => ARTIFACT_DIRECTORY_NODE_VERSION,
            ARTIFACT_FILE_NODE_KIND => ARTIFACT_FILE_NODE_VERSION,
            ARTIFACT_BLOB_KIND => ARTIFACT_BLOB_VERSION,
            ARTIFACT_CHUNK_LIST_KIND => ARTIFACT_CHUNK_LIST_VERSION,
            ARTIFACT_CHUNK_KIND => ARTIFACT_CHUNK_VERSION,
            ARTIFACT_TREE_ROOT_KIND => ARTIFACT_TREE_ROOT_VERSION,
            ARTIFACT_ENVELOPE_KIND => ARTIFACT_ENVELOPE_VERSION,
            _ => {
                return Err(Error::Corrupt(format!(
                    "unknown artifact object kind `{kind}`"
                )))
            }
        };
        if version != i64::from(expected_version) {
            return Err(Error::Corrupt(format!(
                "stored version {version} does not match {kind} version {expected_version}"
            )));
        }
        let actual_id = match kind {
            ARTIFACT_DIRECTORY_NODE_KIND => {
                let node: ArtifactDirectoryNodeV1 = from_cbor(bytes)?;
                for entry in &node.entries {
                    match &entry.target {
                        ArtifactDirectoryEntryTargetV1::Directory { node_id } => {
                            let _: ArtifactDirectoryNodeV1 = self.get_artifact_cas_object(
                                &node_id.0,
                                ARTIFACT_DIRECTORY_NODE_KIND,
                                ARTIFACT_DIRECTORY_NODE_VERSION,
                            )?;
                        }
                        ArtifactDirectoryEntryTargetV1::File { node_id } => {
                            let _: ArtifactFileNodeV1 = self.get_artifact_cas_object(
                                &node_id.0,
                                ARTIFACT_FILE_NODE_KIND,
                                ARTIFACT_FILE_NODE_VERSION,
                            )?;
                        }
                        ArtifactDirectoryEntryTargetV1::Symlink { .. } => {}
                    }
                }
                encode_artifact_directory_node(node)?.0 .0
            }
            ARTIFACT_FILE_NODE_KIND => {
                let node: ArtifactFileNodeV1 = from_cbor(bytes)?;
                self.verify_artifact_file_content(&node)?;
                if logical_bytes as u64 != node.size_bytes {
                    return Err(Error::Corrupt(
                        "file logical byte count disagrees with its node".into(),
                    ));
                }
                encode_artifact_file_node(node)?.0 .0
            }
            ARTIFACT_BLOB_KIND => {
                let blob: ArtifactBlobV1 = from_cbor(bytes)?;
                if logical_bytes as u64 != blob.bytes.len() as u64 {
                    return Err(Error::Corrupt(
                        "blob logical byte count disagrees with its bytes".into(),
                    ));
                }
                encode_artifact_blob(blob)?.0 .0
            }
            ARTIFACT_CHUNK_LIST_KIND => {
                let list: ArtifactChunkListV1 = from_cbor(bytes)?;
                if logical_bytes as u64 != list.file_size_bytes {
                    return Err(Error::Corrupt(
                        "chunk-list logical byte count disagrees with its file size".into(),
                    ));
                }
                for chunk in &list.chunks {
                    let value: ArtifactChunkV1 = self.get_artifact_cas_object(
                        &chunk.chunk_id.0,
                        ARTIFACT_CHUNK_KIND,
                        ARTIFACT_CHUNK_VERSION,
                    )?;
                    if value.bytes.len() as u64 != chunk.size_bytes {
                        return Err(Error::Corrupt(
                            "chunk-list edge size disagrees with its chunk".into(),
                        ));
                    }
                }
                encode_artifact_chunk_list(list)?.0 .0
            }
            ARTIFACT_CHUNK_KIND => {
                let chunk: ArtifactChunkV1 = from_cbor(bytes)?;
                if logical_bytes as u64 != chunk.bytes.len() as u64 {
                    return Err(Error::Corrupt(
                        "chunk logical byte count disagrees with its bytes".into(),
                    ));
                }
                encode_artifact_chunk(chunk)?.0 .0
            }
            ARTIFACT_TREE_ROOT_KIND => {
                let tree: ArtifactTreeRootV1 = from_cbor(bytes)?;
                if logical_bytes as u64 != tree.logical_bytes {
                    return Err(Error::Corrupt(
                        "tree logical byte count disagrees with its root".into(),
                    ));
                }
                encode_artifact_tree_root(tree)?.0 .0
            }
            ARTIFACT_ENVELOPE_KIND => {
                let envelope: ArtifactEnvelopeV1 = from_cbor(bytes)?;
                if logical_bytes != 0 {
                    return Err(Error::Corrupt(
                        "artifact envelope logical byte count must be zero".into(),
                    ));
                }
                encode_artifact_envelope(envelope)?.0 .0
            }
            _ => unreachable!(),
        };
        if actual_id != artifact_id {
            return Err(Error::Corrupt(format!(
                "encoded identity is {actual_id}, not {artifact_id}"
            )));
        }
        Ok(())
    }

    fn validate_artifact_attempt_integrity(&self) -> Result<Vec<String>> {
        let mut errors = Vec::new();
        let mut attempts = self.conn.prepare(
            "SELECT attempt_id,source_root,owner_pid,owner_start_token,phase,status,
                    finished_at
             FROM artifact_construction_attempts ORDER BY attempt_id",
        )?;
        for row in attempts.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<i64>>(6)?,
            ))
        })? {
            let (attempt_id, source_root, owner_pid, owner_token, phase, status, finished_at) =
                row?;
            if self
                .get_object::<WorktreeRoot>(WORKTREE_ROOT_KIND, &ObjectId(source_root))
                .is_err()
            {
                errors.push(format!(
                    "artifact construction attempt {attempt_id} has a missing source root; restore from backup or reinitialize the workspace"
                ));
            }
            if status == "running" {
                let owner_dead = u32::try_from(owner_pid).map_or(true, |pid| {
                    process_start_token_match(pid, &owner_token)
                        == ProcessIdentityMatch::DeadOrMismatch
                });
                if phase == "completed" || finished_at.is_some() || owner_dead {
                    errors.push(format!(
                        "artifact construction attempt {attempt_id} is incomplete or has a dead owner; reopen Trail to run exact-owner recovery"
                    ));
                }
            } else if phase != "completed" || finished_at.is_none() {
                errors.push(format!(
                    "artifact construction attempt {attempt_id} has incoherent terminal phase evidence; restore from backup or reinitialize the workspace"
                ));
            }
        }
        drop(attempts);

        let mut waiters = self.conn.prepare(
            "SELECT w.waiter_id,w.status,a.status
             FROM artifact_construction_waiters w
             JOIN artifact_construction_attempts a ON a.attempt_id=w.attempt_id
             ORDER BY w.waiter_id",
        )?;
        for row in waiters.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })? {
            let (waiter_id, waiter_status, attempt_status) = row?;
            if waiter_status == "waiting" && attempt_status != "running" {
                errors.push(format!(
                    "artifact construction waiter {waiter_id} remains waiting on a terminal attempt; reopen Trail to recover it"
                ));
            }
        }
        Ok(errors)
    }

    fn validate_artifact_resolution_plan_pins(
        &self,
        plan: &ArtifactResolutionPlanV1,
    ) -> Result<()> {
        let _: WorktreeRoot = self.get_object(WORKTREE_ROOT_KIND, &plan.source_root)?;
        let resolved_program = Path::new(&plan.resolved_program);
        if !resolved_program.is_absolute() {
            return Err(Error::InvalidInput(
                "artifact resolver resolved program must be an absolute host path".into(),
            ));
        }
        let actual_identity =
            super::workspace_environment::workspace_tool_identity_for_path(resolved_program)?;
        if actual_identity != plan.executable_identity {
            return Err(Error::InvalidInput(format!(
                "artifact resolver executable `{}` changed after planning",
                plan.program
            )));
        }
        for input in &plan.readable_inputs {
            let entry = self
                .root_file_entry(&plan.source_root, &input.source_path)?
                .ok_or_else(|| {
                    Error::InvalidInput(format!(
                        "artifact resolver input `{}` is absent from pinned source root {}",
                        input.source_path, plan.source_root
                    ))
                })?;
            if entry.content_hash != input.content_hash || entry.size_bytes != input.size_bytes {
                return Err(Error::InvalidInput(format!(
                    "artifact resolver input `{}` changed after planning",
                    input.source_path
                )));
            }
        }
        Ok(())
    }

    fn artifact_resolution_plan_for_fence(
        &self,
        fence: &ArtifactResolutionAttemptFence,
    ) -> Result<ArtifactResolutionPlanV1> {
        let plan_object_id = self
            .conn
            .query_row(
                "SELECT plan_object_id FROM artifact_resolution_attempts
                 WHERE attempt_id=?1 AND owner_generation=?2 AND owner_pid=?3
                   AND owner_start_token=?4 AND status='running'",
                params![
                    fence.attempt_id.0,
                    i64::try_from(fence.owner_generation).map_err(|_| Error::InvalidInput(
                        "artifact resolution owner generation exceeds SQLite range".into()
                    ))?,
                    i64::from(fence.owner_pid),
                    fence.owner_start_token,
                ],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "artifact resolution attempt `{}` lost its exact owner fence",
                    fence.attempt_id
                ))
            })?;
        self.get_object(ARTIFACT_RESOLUTION_PLAN_KIND, &ObjectId(plan_object_id))
    }

    fn put_artifact_resolution_capture(
        &self,
        bytes: &[u8],
        limit: u64,
        redactions: &[Vec<u8>],
    ) -> Result<(Option<ObjectId>, bool)> {
        self.put_artifact_resolution_capture_observed(
            bytes,
            u64::try_from(bytes.len()).unwrap_or(u64::MAX),
            limit,
            redactions,
        )
    }

    fn put_artifact_resolution_capture_observed(
        &self,
        bytes: &[u8],
        original_bytes: u64,
        limit: u64,
        redactions: &[Vec<u8>],
    ) -> Result<(Option<ObjectId>, bool)> {
        if bytes.is_empty() {
            return Ok((None, false));
        }
        let redacted = redact_resolution_bytes(bytes, redactions);
        let limit = usize::try_from(limit).unwrap_or(usize::MAX);
        let truncated = original_bytes > limit as u64 || redacted.len() > limit;
        let capture = ArtifactResolutionCaptureV1 {
            version: ARTIFACT_RESOLUTION_PLAN_VERSION,
            original_bytes,
            truncated,
            bytes: redacted[..redacted.len().min(limit)].to_vec(),
        };
        Ok((
            Some(self.put_object(
                ARTIFACT_RESOLUTION_CAPTURE_KIND,
                ARTIFACT_RESOLUTION_PLAN_VERSION,
                &capture,
            )?),
            truncated,
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn finish_artifact_resolution_attempt_failure_under_write_lock(
        &self,
        fence: &ArtifactResolutionAttemptFence,
        code: &str,
        message: &str,
        authority_evidence: ArtifactResolutionAuthorityEvidenceV1,
        stdout_object_id: Option<ObjectId>,
        stderr_object_id: Option<ObjectId>,
        secret_tainted: bool,
        status: ArtifactResolutionAttemptStatusV1,
    ) -> Result<ArtifactResolutionAttemptReportV1> {
        if !matches!(
            status,
            ArtifactResolutionAttemptStatusV1::Failed
                | ArtifactResolutionAttemptStatusV1::Cancelled
                | ArtifactResolutionAttemptStatusV1::Abandoned
        ) {
            return Err(Error::InvalidInput(
                "artifact resolution failure must use a terminal failure status".into(),
            ));
        }
        let plan = self.artifact_resolution_plan_for_fence(fence)?;
        let receipt = ArtifactResolutionFailureReceiptV1 {
            version: ARTIFACT_RESOLUTION_PLAN_VERSION,
            attempt_id: fence.attempt_id.clone(),
            proposal_key: plan.proposal_key,
            source_root: plan.source_root,
            code: code.to_string(),
            message: message.to_string(),
            authority_evidence: authority_evidence.clone(),
            secret_taint: artifact_secret_taint(secret_tainted, "resolver_credential"),
            stdout_object_id: stdout_object_id.clone(),
            stderr_object_id: stderr_object_id.clone(),
        };
        let receipt_id = self.put_object(
            ARTIFACT_RESOLUTION_FAILURE_KIND,
            ARTIFACT_RESOLUTION_PLAN_VERSION,
            &receipt,
        )?;
        let status_text = artifact_resolution_attempt_status_str(status);
        let updated = self.conn.execute(
            "UPDATE artifact_resolution_attempts
             SET status=?1, authority_evidence_json=?2, stdout_object_id=?3,
                 stderr_object_id=?4, failure_receipt_object_id=?5,
                 failure_code=?6, failure_message=?7, heartbeat_at=?8, finished_at=?8
             WHERE attempt_id=?9 AND owner_generation=?10 AND owner_pid=?11
               AND owner_start_token=?12 AND status='running'",
            params![
                status_text,
                serde_json::to_vec(&authority_evidence)?,
                stdout_object_id.as_ref().map(|id| id.0.as_str()),
                stderr_object_id.as_ref().map(|id| id.0.as_str()),
                receipt_id.0,
                code,
                message,
                now_ts(),
                fence.attempt_id.0,
                i64::try_from(fence.owner_generation).map_err(|_| Error::InvalidInput(
                    "artifact resolution owner generation exceeds SQLite range".into()
                ))?,
                i64::from(fence.owner_pid),
                fence.owner_start_token,
            ],
        )?;
        if updated != 1 {
            return Err(Error::InvalidInput(format!(
                "artifact resolution attempt `{}` lost its exact owner fence",
                fence.attempt_id
            )));
        }
        self.artifact_resolution_attempt(&fence.attempt_id)
    }

    fn get_artifact_cas_object<T: serde::de::DeserializeOwned>(
        &self,
        artifact_id: &str,
        kind: &'static str,
        version: u16,
    ) -> Result<T> {
        let (object_id, stored_kind, stored_version) = self.conn.query_row(
            "SELECT object_id, kind, version FROM artifact_objects WHERE artifact_id=?1",
            params![artifact_id],
            |row| {
                Ok((
                    ObjectId(row.get::<_, String>(0)?),
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )?;
        if stored_kind != kind || stored_version != i64::from(version) {
            return Err(Error::Corrupt(format!(
                "artifact object `{artifact_id}` has kind/version {stored_kind}/{stored_version}, expected {kind}/{version}"
            )));
        }
        self.get_object(kind, &object_id)
    }

    fn put_artifact_cas_object<T: Serialize>(
        &self,
        artifact_id: &str,
        kind: &'static str,
        version: u16,
        logical_bytes: u64,
        value: &T,
    ) -> Result<ObjectId> {
        let canonical_bytes = cbor(value)?;
        let logical_bytes = i64::try_from(logical_bytes).map_err(|_| {
            Error::InvalidInput("artifact logical byte count exceeds SQLite range".into())
        })?;
        self.conn.execute_batch("SAVEPOINT trail_artifact_object")?;
        let publication = (|| -> Result<ObjectId> {
            let object_id = self.put_object(kind, version, value)?;
            let stored = self.conn.query_row(
                "SELECT kind, version, bytes FROM objects WHERE object_id=?1",
                params![object_id.0],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, i64>(1)?,
                        row.get::<_, Vec<u8>>(2)?,
                    ))
                },
            )?;
            if stored.0 != kind || stored.1 != i64::from(version) || stored.2 != canonical_bytes {
                return Err(Error::Corrupt(format!(
                    "content-addressed object {} conflicts with artifact `{artifact_id}`",
                    object_id
                )));
            }
            let existing = self
                .conn
                .query_row(
                    "SELECT object_id, kind, version, logical_bytes
                     FROM artifact_objects WHERE artifact_id=?1",
                    params![artifact_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                            row.get::<_, i64>(3)?,
                        ))
                    },
                )
                .optional()?;
            if let Some((existing_object, existing_kind, existing_version, existing_bytes)) =
                existing
            {
                if existing_object != object_id.0
                    || existing_kind != kind
                    || existing_version != i64::from(version)
                    || existing_bytes != logical_bytes
                {
                    return Err(Error::Corrupt(format!(
                        "artifact ID `{artifact_id}` resolves to conflicting object evidence"
                    )));
                }
                return Ok(object_id);
            }
            self.conn.execute(
                "INSERT INTO artifact_objects(
                    artifact_id, object_id, kind, version, logical_bytes, created_at
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    artifact_id,
                    object_id.0,
                    kind,
                    i64::from(version),
                    logical_bytes,
                    now_ts(),
                ],
            )?;
            Ok(object_id)
        })();
        match publication {
            Ok(object_id) => {
                self.conn
                    .execute_batch("RELEASE SAVEPOINT trail_artifact_object")?;
                Ok(object_id)
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT trail_artifact_object;
                     RELEASE SAVEPOINT trail_artifact_object",
                );
                Err(error)
            }
        }
    }

    fn ingest_artifact_file_bytes(&self, bytes: &[u8], mode: u32) -> Result<ArtifactFileId> {
        self.ingest_artifact_file_bytes_with_path(bytes, mode, None, ArtifactSecretPolicy::Strict)
    }

    fn ingest_artifact_file_bytes_with_path(
        &self,
        bytes: &[u8],
        mode: u32,
        relative_path: Option<&str>,
        secret_policy: ArtifactSecretPolicy,
    ) -> Result<ArtifactFileId> {
        if mode & !0o777 != 0 {
            return Err(Error::InvalidInput(format!(
                "artifact file mode {mode:o} contains unsupported bits"
            )));
        }
        validate_artifact_secret_policy(bytes, relative_path, secret_policy)?;
        let complete_hash = sha256_hex(bytes);
        let content = if bytes.len() <= ARTIFACT_WHOLE_BLOB_MAX_BYTES {
            let blob = ArtifactBlobV1 {
                version: ARTIFACT_BLOB_VERSION,
                content_sha256: complete_hash.clone(),
                bytes: bytes.to_vec(),
            };
            let (blob_id, _) = encode_artifact_blob(blob.clone())?;
            self.put_artifact_cas_object(
                &blob_id.0,
                ARTIFACT_BLOB_KIND,
                ARTIFACT_BLOB_VERSION,
                bytes.len() as u64,
                &blob,
            )?;
            ArtifactFileContentV1::Blob { blob_id }
        } else {
            let mut chunks = Vec::new();
            for boundary in fastcdc::v2020::FastCDC::new(
                bytes,
                ARTIFACT_CHUNK_MIN_BYTES,
                ARTIFACT_CHUNK_AVERAGE_BYTES,
                ARTIFACT_CHUNK_MAX_BYTES,
            ) {
                let end = boundary
                    .offset
                    .checked_add(boundary.length)
                    .ok_or_else(|| {
                        Error::InvalidInput("artifact chunk boundary overflow".into())
                    })?;
                let chunk_bytes = bytes.get(boundary.offset..end).ok_or_else(|| {
                    Error::Corrupt("FastCDC returned an out-of-range artifact chunk".into())
                })?;
                let chunk = ArtifactChunkV1 {
                    version: ARTIFACT_CHUNK_VERSION,
                    content_sha256: sha256_hex(chunk_bytes),
                    bytes: chunk_bytes.to_vec(),
                };
                let (chunk_id, _) = encode_artifact_chunk(chunk.clone())?;
                self.put_artifact_cas_object(
                    &chunk_id.0,
                    ARTIFACT_CHUNK_KIND,
                    ARTIFACT_CHUNK_VERSION,
                    chunk_bytes.len() as u64,
                    &chunk,
                )?;
                chunks.push(ArtifactChunkRefV1 {
                    chunk_id,
                    size_bytes: chunk_bytes.len() as u64,
                });
            }
            let chunk_list = ArtifactChunkListV1 {
                version: ARTIFACT_CHUNK_LIST_VERSION,
                algorithm: "fastcdc-v1".into(),
                file_size_bytes: bytes.len() as u64,
                file_sha256: complete_hash.clone(),
                chunks,
            };
            let (chunk_list_id, _) = encode_artifact_chunk_list(chunk_list.clone())?;
            self.put_artifact_cas_object(
                &chunk_list_id.0,
                ARTIFACT_CHUNK_LIST_KIND,
                ARTIFACT_CHUNK_LIST_VERSION,
                bytes.len() as u64,
                &chunk_list,
            )?;
            ArtifactFileContentV1::Chunks { chunk_list_id }
        };
        let file = ArtifactFileNodeV1 {
            version: ARTIFACT_FILE_NODE_VERSION,
            mode,
            executable: mode & 0o111 != 0,
            size_bytes: bytes.len() as u64,
            content_sha256: complete_hash,
            content,
        };
        let (file_id, _) = encode_artifact_file_node(file.clone())?;
        self.put_artifact_cas_object(
            &file_id.0,
            ARTIFACT_FILE_NODE_KIND,
            ARTIFACT_FILE_NODE_VERSION,
            bytes.len() as u64,
            &file,
        )?;
        Ok(file_id)
    }

    fn ingest_artifact_file_path(
        &self,
        path: &Path,
        relative_path: &str,
        mode: u32,
        secret_policy: ArtifactSecretPolicy,
    ) -> Result<ArtifactFileId> {
        let before = fs::symlink_metadata(path)?;
        if !before.is_file() {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().into_owned(),
                reason: "artifact file input changed type during ingestion".into(),
            });
        }
        if before.len() <= ARTIFACT_WHOLE_BLOB_MAX_BYTES as u64 {
            let bytes = fs::read(path)?;
            let after = fs::symlink_metadata(path)?;
            ensure_artifact_file_unchanged(path, &before, &after, bytes.len() as u64)?;
            return self.ingest_artifact_file_bytes_with_path(
                &bytes,
                mode,
                Some(relative_path),
                secret_policy,
            );
        }

        let mut complete_hasher = Sha256::new();
        let mut chunks = Vec::new();
        for item in fastcdc::v2020::StreamCDC::new(
            File::open(path)?,
            ARTIFACT_CHUNK_MIN_BYTES,
            ARTIFACT_CHUNK_AVERAGE_BYTES,
            ARTIFACT_CHUNK_MAX_BYTES,
        ) {
            let boundary = item.map_err(|error| {
                Error::InvalidInput(format!(
                    "cannot chunk artifact file `{}`: {error}",
                    path.display()
                ))
            })?;
            validate_artifact_secret_policy(&boundary.data, Some(relative_path), secret_policy)?;
            complete_hasher.update(&boundary.data);
            let chunk = ArtifactChunkV1 {
                version: ARTIFACT_CHUNK_VERSION,
                content_sha256: sha256_hex(&boundary.data),
                bytes: boundary.data,
            };
            let (chunk_id, _) = encode_artifact_chunk(chunk.clone())?;
            self.put_artifact_cas_object(
                &chunk_id.0,
                ARTIFACT_CHUNK_KIND,
                ARTIFACT_CHUNK_VERSION,
                chunk.bytes.len() as u64,
                &chunk,
            )?;
            chunks.push(ArtifactChunkRefV1 {
                chunk_id,
                size_bytes: chunk.bytes.len() as u64,
            });
        }
        let after = fs::symlink_metadata(path)?;
        let streamed_bytes = chunks
            .iter()
            .try_fold(0u64, |total, chunk| total.checked_add(chunk.size_bytes));
        ensure_artifact_file_unchanged(
            path,
            &before,
            &after,
            streamed_bytes
                .ok_or_else(|| Error::InvalidInput("artifact file size overflow".into()))?,
        )?;
        let complete_hash = hex::encode(complete_hasher.finalize());
        let chunk_list = ArtifactChunkListV1 {
            version: ARTIFACT_CHUNK_LIST_VERSION,
            algorithm: "fastcdc-v1".into(),
            file_size_bytes: before.len(),
            file_sha256: complete_hash.clone(),
            chunks,
        };
        let (chunk_list_id, _) = encode_artifact_chunk_list(chunk_list.clone())?;
        self.put_artifact_cas_object(
            &chunk_list_id.0,
            ARTIFACT_CHUNK_LIST_KIND,
            ARTIFACT_CHUNK_LIST_VERSION,
            before.len(),
            &chunk_list,
        )?;
        let node = ArtifactFileNodeV1 {
            version: ARTIFACT_FILE_NODE_VERSION,
            mode,
            executable: mode & 0o111 != 0,
            size_bytes: before.len(),
            content_sha256: complete_hash,
            content: ArtifactFileContentV1::Chunks { chunk_list_id },
        };
        let (file_id, _) = encode_artifact_file_node(node.clone())?;
        self.put_artifact_cas_object(
            &file_id.0,
            ARTIFACT_FILE_NODE_KIND,
            ARTIFACT_FILE_NODE_VERSION,
            before.len(),
            &node,
        )?;
        Ok(file_id)
    }

    fn ingest_artifact_tree(&self, source: &Path) -> Result<(ArtifactTreeId, ArtifactTreeRootV1)> {
        let _lock = self.acquire_write_lock()?;
        self.ingest_artifact_tree_under_write_lock(source)
    }

    pub(crate) fn ingest_artifact_tree_under_write_lock(
        &self,
        source: &Path,
    ) -> Result<(ArtifactTreeId, ArtifactTreeRootV1)> {
        self.ingest_artifact_tree_under_write_lock_with_secret_policy(
            source,
            ArtifactSecretPolicy::Strict,
        )
    }

    pub(crate) fn ingest_artifact_tree_under_write_lock_with_secret_policy(
        &self,
        source: &Path,
        secret_policy: ArtifactSecretPolicy,
    ) -> Result<(ArtifactTreeId, ArtifactTreeRootV1)> {
        let root_before = fs::symlink_metadata(source)?;
        if root_before.file_type().is_symlink() || !root_before.is_dir() {
            return Err(Error::InvalidPath {
                path: source.to_string_lossy().into_owned(),
                reason: "artifact tree source must be a real directory".into(),
            });
        }
        let mut directories = BTreeMap::<String, Vec<ArtifactDirectoryEntryV1>>::new();
        directories.insert(String::new(), Vec::new());
        let mut entry_count = 0u64;
        let mut logical_bytes = 0u64;
        let mut case_paths = Vec::new();

        for entry in walkdir::WalkDir::new(source)
            .follow_links(false)
            .max_depth(MAX_ARTIFACT_TREE_DEPTH + 1)
        {
            let entry = entry.map_err(|error| Error::InvalidInput(error.to_string()))?;
            if entry.depth() == 0 {
                continue;
            }
            if entry.depth() > MAX_ARTIFACT_TREE_DEPTH {
                return Err(Error::InvalidInput(format!(
                    "artifact tree exceeds maximum depth {MAX_ARTIFACT_TREE_DEPTH}"
                )));
            }
            entry_count = entry_count
                .checked_add(1)
                .ok_or_else(|| Error::InvalidInput("artifact tree entry count overflow".into()))?;
            if entry_count > MAX_ARTIFACT_TREE_ENTRIES {
                return Err(Error::InvalidInput(format!(
                    "artifact tree exceeds {MAX_ARTIFACT_TREE_ENTRIES} entries"
                )));
            }
            let relative = entry
                .path()
                .strip_prefix(source)
                .map_err(|_| Error::InvalidPath {
                    path: entry.path().to_string_lossy().into_owned(),
                    reason: "artifact walk escaped its source root".into(),
                })?;
            let relative = relative.to_str().ok_or_else(|| Error::InvalidPath {
                path: relative.to_string_lossy().into_owned(),
                reason: "artifact paths must be valid Unicode".into(),
            })?;
            let relative = normalize_relative_path(relative)?;
            case_paths.push(relative.clone());
            let (parent, name) = relative.rsplit_once('/').unwrap_or(("", &relative));
            validate_artifact_entry_name(name)?;
            directories.entry(parent.to_string()).or_default();
            let file_type = entry.file_type();
            validate_artifact_metadata_policy(entry.path(), &fs::symlink_metadata(entry.path())?)?;
            if file_type.is_dir() {
                directories.entry(relative).or_default();
            } else if file_type.is_file() {
                let metadata = fs::symlink_metadata(entry.path())?;
                logical_bytes = logical_bytes.checked_add(metadata.len()).ok_or_else(|| {
                    Error::InvalidInput("artifact tree logical byte count overflow".into())
                })?;
                if logical_bytes > MAX_ARTIFACT_TREE_LOGICAL_BYTES {
                    return Err(Error::InvalidInput(format!(
                        "artifact tree exceeds {MAX_ARTIFACT_TREE_LOGICAL_BYTES} logical bytes"
                    )));
                }
                let file_id = self.ingest_artifact_file_path(
                    entry.path(),
                    &relative,
                    normalized_artifact_file_mode(&metadata),
                    secret_policy,
                )?;
                directories
                    .get_mut(parent)
                    .unwrap()
                    .push(ArtifactDirectoryEntryV1 {
                        name: name.to_string(),
                        target: ArtifactDirectoryEntryTargetV1::File { node_id: file_id },
                    });
            } else if file_type.is_symlink() {
                let target = fs::read_link(entry.path())?;
                let target = target.to_str().ok_or_else(|| Error::InvalidPath {
                    path: entry.path().to_string_lossy().into_owned(),
                    reason: "artifact symlink targets must be valid Unicode".into(),
                })?;
                validate_artifact_symlink_within_tree(parent, target)?;
                directories
                    .get_mut(parent)
                    .unwrap()
                    .push(ArtifactDirectoryEntryV1 {
                        name: name.to_string(),
                        target: ArtifactDirectoryEntryTargetV1::Symlink {
                            target: target.to_string(),
                        },
                    });
            } else {
                return Err(Error::InvalidPath {
                    path: entry.path().to_string_lossy().into_owned(),
                    reason: "artifact trees support only directories, regular files, and confined symlinks".into(),
                });
            }
        }
        validate_no_case_fold_collisions(&case_paths)?;

        let mut paths = directories.keys().cloned().collect::<Vec<_>>();
        paths.sort_by(|left, right| {
            right
                .split('/')
                .count()
                .cmp(&left.split('/').count())
                .then_with(|| left.cmp(right))
        });
        for path in paths.into_iter().filter(|path| !path.is_empty()) {
            let node = canonical_artifact_directory_node(ArtifactDirectoryNodeV1 {
                version: ARTIFACT_DIRECTORY_NODE_VERSION,
                entries: directories.remove(&path).unwrap_or_default(),
            })?;
            let (node_id, _) = encode_artifact_directory_node(node.clone())?;
            self.put_artifact_cas_object(
                &node_id.0,
                ARTIFACT_DIRECTORY_NODE_KIND,
                ARTIFACT_DIRECTORY_NODE_VERSION,
                0,
                &node,
            )?;
            let (parent, name) = path.rsplit_once('/').unwrap_or(("", path.as_str()));
            directories
                .get_mut(parent)
                .unwrap()
                .push(ArtifactDirectoryEntryV1 {
                    name: name.to_string(),
                    target: ArtifactDirectoryEntryTargetV1::Directory { node_id },
                });
        }
        let root_node = canonical_artifact_directory_node(ArtifactDirectoryNodeV1 {
            version: ARTIFACT_DIRECTORY_NODE_VERSION,
            entries: directories.remove("").unwrap_or_default(),
        })?;
        let (root_directory_id, _) = encode_artifact_directory_node(root_node.clone())?;
        self.put_artifact_cas_object(
            &root_directory_id.0,
            ARTIFACT_DIRECTORY_NODE_KIND,
            ARTIFACT_DIRECTORY_NODE_VERSION,
            logical_bytes,
            &root_node,
        )?;
        let tree = ArtifactTreeRootV1 {
            version: ARTIFACT_TREE_ROOT_VERSION,
            root_directory_id,
            logical_bytes,
            entry_count,
            path_normalizer: "trail-paths/v1".into(),
        };
        let (tree_id, _) = encode_artifact_tree_root(tree.clone())?;
        self.put_artifact_cas_object(
            &tree_id.0,
            ARTIFACT_TREE_ROOT_KIND,
            ARTIFACT_TREE_ROOT_VERSION,
            logical_bytes,
            &tree,
        )?;
        let root_after = fs::symlink_metadata(source)?;
        if !same_artifact_metadata(&root_before, &root_after) {
            return Err(Error::InvalidInput(
                "artifact tree root changed during ingestion".into(),
            ));
        }
        Ok((tree_id, tree))
    }

    pub(crate) fn artifact_tree_flat_entries(
        &self,
        tree_id: &ArtifactTreeId,
    ) -> Result<BTreeMap<String, ArtifactFlatEntry>> {
        let tree: ArtifactTreeRootV1 = self.get_artifact_cas_object(
            &tree_id.0,
            ARTIFACT_TREE_ROOT_KIND,
            ARTIFACT_TREE_ROOT_VERSION,
        )?;
        let (actual_tree_id, _) = encode_artifact_tree_root(tree.clone())?;
        if &actual_tree_id != tree_id {
            return Err(Error::Corrupt(format!(
                "artifact tree root `{tree_id}` has conflicting encoded identity"
            )));
        }
        let mut entries = BTreeMap::new();
        let mut visiting = BTreeSet::new();
        self.flatten_artifact_directory(
            &tree.root_directory_id,
            "",
            0,
            &mut visiting,
            &mut entries,
        )?;
        let logical_bytes = entries
            .values()
            .try_fold(0u64, |total, entry| total.checked_add(entry.size_bytes));
        if entries.len() as u64 != tree.entry_count || logical_bytes != Some(tree.logical_bytes) {
            return Err(Error::Corrupt(format!(
                "artifact tree `{tree_id}` count or logical-byte edge is invalid"
            )));
        }
        Ok(entries)
    }

    pub(crate) fn artifact_tree_object_ids(
        &self,
        tree_id: &ArtifactTreeId,
    ) -> Result<BTreeSet<String>> {
        let tree: ArtifactTreeRootV1 = self.get_artifact_cas_object(
            &tree_id.0,
            ARTIFACT_TREE_ROOT_KIND,
            ARTIFACT_TREE_ROOT_VERSION,
        )?;
        let mut objects =
            BTreeSet::from([self.artifact_backing_object_id(&tree_id.0, ARTIFACT_TREE_ROOT_KIND)?]);
        let mut visited_directories = BTreeSet::new();
        self.collect_artifact_directory_object_ids(
            &tree.root_directory_id,
            0,
            &mut visited_directories,
            &mut objects,
        )?;
        Ok(objects)
    }

    pub(crate) fn artifact_envelope_object_ids(
        &self,
        envelope_id: &ArtifactEnvelopeId,
    ) -> Result<BTreeSet<String>> {
        let envelope: ArtifactEnvelopeV1 = self.get_artifact_cas_object(
            &envelope_id.0,
            ARTIFACT_ENVELOPE_KIND,
            ARTIFACT_ENVELOPE_VERSION,
        )?;
        let mut objects = self.artifact_tree_object_ids(&envelope.tree_root_id)?;
        objects.insert(self.artifact_backing_object_id(&envelope_id.0, ARTIFACT_ENVELOPE_KIND)?);
        objects.extend(
            envelope
                .validation_receipt_ids
                .into_iter()
                .map(|object_id| object_id.0),
        );
        if let Some(snapshot_id) = envelope.resolution_snapshot_id {
            self.collect_artifact_resolution_snapshot_object_ids(&snapshot_id, &mut objects)?;
        }
        let mut statement = self.conn.prepare(
            "SELECT object_id FROM artifact_attestations
             WHERE envelope_id=?1 ORDER BY attestation_id",
        )?;
        for row in statement.query_map(params![envelope_id.0], |row| row.get::<_, String>(0))? {
            objects.insert(row?);
        }
        Ok(objects)
    }

    fn artifact_backing_object_id(&self, artifact_id: &str, expected_kind: &str) -> Result<String> {
        let (object_id, kind) = self
            .conn
            .query_row(
                "SELECT object_id,kind FROM artifact_objects WHERE artifact_id=?1",
                params![artifact_id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()?
            .ok_or_else(|| {
                Error::Corrupt(format!(
                    "artifact object `{artifact_id}` is missing its backing object"
                ))
            })?;
        if kind != expected_kind {
            return Err(Error::Corrupt(format!(
                "artifact object `{artifact_id}` has kind {kind}, expected {expected_kind}"
            )));
        }
        Ok(object_id)
    }

    fn collect_artifact_directory_object_ids(
        &self,
        directory_id: &ArtifactTreeId,
        depth: usize,
        visited_directories: &mut BTreeSet<ArtifactTreeId>,
        objects: &mut BTreeSet<String>,
    ) -> Result<()> {
        if depth > MAX_ARTIFACT_TREE_DEPTH {
            return Err(Error::Corrupt(
                "artifact accounting exceeded the directory-depth bound".into(),
            ));
        }
        if !visited_directories.insert(directory_id.clone()) {
            return Ok(());
        }
        objects.insert(
            self.artifact_backing_object_id(&directory_id.0, ARTIFACT_DIRECTORY_NODE_KIND)?,
        );
        let directory: ArtifactDirectoryNodeV1 = self.get_artifact_cas_object(
            &directory_id.0,
            ARTIFACT_DIRECTORY_NODE_KIND,
            ARTIFACT_DIRECTORY_NODE_VERSION,
        )?;
        for entry in directory.entries {
            match entry.target {
                ArtifactDirectoryEntryTargetV1::Directory { node_id } => {
                    self.collect_artifact_directory_object_ids(
                        &node_id,
                        depth + 1,
                        visited_directories,
                        objects,
                    )?;
                }
                ArtifactDirectoryEntryTargetV1::File { node_id } => {
                    objects.insert(
                        self.artifact_backing_object_id(&node_id.0, ARTIFACT_FILE_NODE_KIND)?,
                    );
                    let file: ArtifactFileNodeV1 = self.get_artifact_cas_object(
                        &node_id.0,
                        ARTIFACT_FILE_NODE_KIND,
                        ARTIFACT_FILE_NODE_VERSION,
                    )?;
                    match file.content {
                        ArtifactFileContentV1::Blob { blob_id } => {
                            objects.insert(
                                self.artifact_backing_object_id(&blob_id.0, ARTIFACT_BLOB_KIND)?,
                            );
                        }
                        ArtifactFileContentV1::Chunks { chunk_list_id } => {
                            objects.insert(self.artifact_backing_object_id(
                                &chunk_list_id.0,
                                ARTIFACT_CHUNK_LIST_KIND,
                            )?);
                            let list: ArtifactChunkListV1 = self.get_artifact_cas_object(
                                &chunk_list_id.0,
                                ARTIFACT_CHUNK_LIST_KIND,
                                ARTIFACT_CHUNK_LIST_VERSION,
                            )?;
                            for chunk in list.chunks {
                                objects.insert(self.artifact_backing_object_id(
                                    &chunk.chunk_id.0,
                                    ARTIFACT_CHUNK_KIND,
                                )?);
                            }
                        }
                    }
                }
                ArtifactDirectoryEntryTargetV1::Symlink { .. } => {}
            }
        }
        Ok(())
    }

    fn collect_artifact_resolution_snapshot_object_ids(
        &self,
        snapshot_id: &ObjectId,
        objects: &mut BTreeSet<String>,
    ) -> Result<()> {
        let mut pending = BTreeSet::from([snapshot_id.clone()]);
        let mut visited = BTreeSet::new();
        while let Some(snapshot_id) = pending.pop_first() {
            if !visited.insert(snapshot_id.clone()) {
                continue;
            }
            if visited.len() > MAX_RESOLUTION_PREDECESSORS {
                return Err(Error::Corrupt(
                    "artifact resolution predecessor graph exceeds its bound".into(),
                ));
            }
            let snapshot: ArtifactResolutionSnapshotV1 =
                self.get_object(ARTIFACT_RESOLUTION_SNAPSHOT_KIND, &snapshot_id)?;
            objects.insert(snapshot_id.0);
            objects.insert(snapshot.content_object_id.0);
            pending.extend(snapshot.predecessor_snapshot_id);
        }
        Ok(())
    }

    pub(crate) fn artifact_storage_accounting(
        &self,
        view_id: Option<&str>,
        lane_private_bytes: u64,
        demand_loaded_bytes: u64,
        reclaimable_bytes: u64,
        measured_unknown_bytes: u64,
    ) -> Result<ArtifactStorageAccountingReport> {
        const MAX_ACCOUNTING_ENVELOPES: usize = 10_000;
        const MAX_ACCOUNTING_TREES: usize = 10_000;
        const MAX_ACCOUNTING_OBJECTS: usize = 10_000_000;
        const MAX_ACCOUNTING_OBJECT_REFERENCES: usize = 10_000_000;

        let integrity_errors = self.validate_artifact_cas_integrity()?;
        if !integrity_errors.is_empty() {
            return Err(Error::Corrupt(integrity_errors.join("; ")));
        }
        let envelopes = {
            let mut statement = self.conn.prepare(
                "SELECT envelope_id,tree_root_id FROM artifact_envelopes
                 ORDER BY envelope_id",
            )?;
            statement
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        if envelopes.len() > MAX_ACCOUNTING_ENVELOPES {
            return Err(Error::InvalidInput(format!(
                "artifact accounting contains {} envelopes; maximum is {MAX_ACCOUNTING_ENVELOPES}",
                envelopes.len()
            )));
        }

        let mut envelope_graphs = BTreeMap::<String, BTreeSet<String>>::new();
        let mut object_reference_counts = BTreeMap::<String, u64>::new();
        let mut reference_count = 0_usize;
        for (envelope_id, _) in &envelopes {
            let envelope_id = ArtifactEnvelopeId::parse(envelope_id.clone()).map_err(|error| {
                Error::Corrupt(format!("invalid artifact envelope ID: {error}"))
            })?;
            let graph = self.artifact_envelope_object_ids(&envelope_id)?;
            reference_count = reference_count.checked_add(graph.len()).ok_or_else(|| {
                Error::InvalidInput("artifact accounting reference count overflowed".into())
            })?;
            if reference_count > MAX_ACCOUNTING_OBJECT_REFERENCES {
                return Err(Error::InvalidInput(format!(
                    "artifact accounting contains more than {MAX_ACCOUNTING_OBJECT_REFERENCES} object references"
                )));
            }
            for object_id in &graph {
                let count = object_reference_counts
                    .entry(object_id.clone())
                    .or_default();
                *count = count.saturating_add(1);
            }
            envelope_graphs.insert(envelope_id.0, graph);
        }

        let (selected_envelopes, mut selected_trees) = if let Some(view_id) = view_id {
            let mut statement = self.conn.prepare(
                "SELECT b.envelope_id,b.tree_root_id
                 FROM environment_view_generations v
                 JOIN artifact_generation_bindings b ON b.generation_id=v.generation_id
                 WHERE v.view_id=?1
                 UNION
                 SELECT s.envelope_id,s.tree_root_id
                 FROM workspace_view_layers l
                 JOIN workspace_layer_artifact_shadows s ON s.layer_id=l.layer_id
                 WHERE l.view_id=?1
                 ORDER BY envelope_id,tree_root_id",
            )?;
            let rows = statement
                .query_map(params![view_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            (
                rows.iter()
                    .map(|(envelope_id, _)| envelope_id.clone())
                    .collect::<BTreeSet<_>>(),
                rows.into_iter()
                    .map(|(_, tree_root_id)| tree_root_id)
                    .collect::<BTreeSet<_>>(),
            )
        } else {
            (
                envelopes
                    .iter()
                    .map(|(envelope_id, _)| envelope_id.clone())
                    .collect::<BTreeSet<_>>(),
                envelopes
                    .iter()
                    .map(|(_, tree_root_id)| tree_root_id.clone())
                    .collect::<BTreeSet<_>>(),
            )
        };

        let mut selected_objects = BTreeSet::<String>::new();
        for envelope_id in &selected_envelopes {
            let graph = envelope_graphs.get(envelope_id).ok_or_else(|| {
                Error::Corrupt(format!(
                    "selected artifact envelope `{envelope_id}` is missing its accounting graph"
                ))
            })?;
            selected_objects.extend(graph.iter().cloned());
        }
        if view_id.is_none() {
            let mut statement = self
                .conn
                .prepare("SELECT object_id FROM artifact_objects ORDER BY object_id")?;
            for row in statement.query_map([], |row| row.get::<_, String>(0))? {
                selected_objects.insert(row?);
            }
            let mut statement = self.conn.prepare(
                "SELECT object_id FROM objects WHERE kind IN (
                    ?1,?2,?3,?4,?5,?6,?7
                 ) ORDER BY object_id",
            )?;
            for row in statement.query_map(
                params![
                    ARTIFACT_RESOLUTION_SNAPSHOT_KIND,
                    ARTIFACT_RESOLUTION_CONTENT_KIND,
                    ARTIFACT_RESOLUTION_PLAN_KIND,
                    ARTIFACT_RESOLUTION_CAPTURE_KIND,
                    ARTIFACT_RESOLUTION_FAILURE_KIND,
                    ARTIFACT_DIVERGENCE_EVIDENCE_KIND,
                    ARTIFACT_VALIDATION_RECEIPT_KIND,
                ],
                |row| row.get::<_, String>(0),
            )? {
                selected_objects.insert(row?);
            }
            let mut statement = self.conn.prepare(
                "SELECT artifact_id FROM artifact_objects
                 WHERE kind=?1 ORDER BY artifact_id",
            )?;
            for row in statement.query_map(params![ARTIFACT_TREE_ROOT_KIND], |row| {
                row.get::<_, String>(0)
            })? {
                selected_trees.insert(row?);
            }
        }
        if selected_trees.len() > MAX_ACCOUNTING_TREES {
            return Err(Error::InvalidInput(format!(
                "artifact accounting contains {} selected trees; maximum is {MAX_ACCOUNTING_TREES}",
                selected_trees.len()
            )));
        }
        if selected_objects.len() > MAX_ACCOUNTING_OBJECTS {
            return Err(Error::InvalidInput(format!(
                "artifact accounting contains {} selected objects; maximum is {MAX_ACCOUNTING_OBJECTS}",
                selected_objects.len()
            )));
        }

        let mut unique_authoritative_bytes = 0_u64;
        let mut cross_artifact_shared_bytes = 0_u64;
        for chunk in selected_objects.iter().collect::<Vec<_>>().chunks(512) {
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT object_id,size_bytes FROM objects WHERE object_id IN ({placeholders})"
            );
            let mut statement = self.conn.prepare(&sql)?;
            let rows = statement.query_map(
                params_from_iter(chunk.iter().map(|object_id| object_id.as_str())),
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
            )?;
            for row in rows {
                let (object_id, size_bytes) = row?;
                let size_bytes = u64::try_from(size_bytes).map_err(|_| {
                    Error::Corrupt(format!(
                        "artifact accounting object `{object_id}` has negative bytes"
                    ))
                })?;
                if object_reference_counts
                    .get(&object_id)
                    .copied()
                    .unwrap_or(1)
                    > 1
                {
                    cross_artifact_shared_bytes =
                        cross_artifact_shared_bytes.saturating_add(size_bytes);
                } else {
                    unique_authoritative_bytes =
                        unique_authoritative_bytes.saturating_add(size_bytes);
                }
            }
        }

        let mut logical_bytes = 0_u64;
        for tree_root_id in &selected_trees {
            let bytes = self.conn.query_row(
                "SELECT logical_bytes FROM artifact_objects
                 WHERE artifact_id=?1 AND kind=?2",
                params![tree_root_id, ARTIFACT_TREE_ROOT_KIND],
                |row| row.get::<_, i64>(0),
            )?;
            logical_bytes = logical_bytes.saturating_add(u64::try_from(bytes).map_err(|_| {
                Error::Corrupt(format!(
                    "artifact tree `{tree_root_id}` has negative logical bytes"
                ))
            })?);
        }

        let materialized_bytes = self.artifact_materialized_bytes(view_id, &selected_trees)?;
        Ok(ArtifactStorageAccountingReport {
            logical_bytes,
            unique_authoritative_bytes,
            cross_artifact_shared_bytes,
            materialized_bytes,
            lane_private_bytes,
            prefetched_bytes: 0,
            demand_loaded_bytes,
            reclaimable_bytes,
            unknown_bytes: measured_unknown_bytes,
            accounting: format!(
                "scope={};axes=logical|authoritative|physical|disposition;authoritative=encoded-cbor-bytes-deduplicated;filesystem=allocated-blocks-or-file-size-estimate;reclaimable=overlapping-disposition;prefetch=os-page-cache-excluded",
                if view_id.is_some() { "lane" } else { "workspace" }
            ),
        })
    }

    /// Return workspace-wide CAS accounting without requiring a lane view.
    pub fn workspace_artifact_space(&self) -> Result<ArtifactSpaceReportV1> {
        let envelope_count =
            self.conn
                .query_row("SELECT COUNT(*) FROM artifact_envelopes", [], |row| {
                    row.get::<_, i64>(0)
                })?;
        let active_quarantine_count = self.conn.query_row(
            "SELECT COUNT(*) FROM artifact_quarantines WHERE state='active'",
            [],
            |row| row.get::<_, i64>(0),
        )?;
        Ok(ArtifactSpaceReportV1 {
            scope: "workspace".into(),
            envelope_count: u64::try_from(envelope_count)
                .map_err(|_| Error::Corrupt("negative artifact envelope count".into()))?,
            active_quarantine_count: u64::try_from(active_quarantine_count)
                .map_err(|_| Error::Corrupt("negative artifact quarantine count".into()))?,
            storage: self.artifact_storage_accounting(None, 0, 0, 0, 0)?,
        })
    }

    pub(crate) fn artifact_envelope_ids(&self) -> Result<Vec<String>> {
        let mut statement = self
            .conn
            .prepare("SELECT envelope_id FROM artifact_envelopes ORDER BY envelope_id")?;
        statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)
    }

    /// Traverse the durable object graph reachable from one artifact envelope.
    /// The report is a bounded summary; object identifiers remain private storage
    /// details and are not expanded into an unbounded public payload.
    pub fn artifact_content_reachability(
        &self,
        envelope_id: &ArtifactEnvelopeId,
    ) -> Result<ArtifactContentReachabilityReportV1> {
        let tree_root_id = self.artifact_envelope_tree_id(envelope_id)?;
        let object_ids = self.artifact_envelope_object_ids(envelope_id)?;
        let objects = self.artifact_object_storage_rows(&object_ids)?;
        let mut kinds = BTreeMap::<String, (u64, u64)>::new();
        let mut encoded_bytes = 0_u64;
        for (_, kind, size_bytes) in objects.values() {
            encoded_bytes = encoded_bytes.saturating_add(*size_bytes);
            let entry = kinds.entry(kind.clone()).or_default();
            entry.0 = entry.0.saturating_add(1);
            entry.1 = entry.1.saturating_add(*size_bytes);
        }
        let tree: ArtifactTreeRootV1 = self.get_artifact_cas_object(
            &tree_root_id.0,
            ARTIFACT_TREE_ROOT_KIND,
            ARTIFACT_TREE_ROOT_VERSION,
        )?;
        Ok(ArtifactContentReachabilityReportV1 {
            envelope_id: envelope_id.clone(),
            tree_root_id,
            object_count: object_ids.len() as u64,
            encoded_bytes,
            logical_bytes: tree.logical_bytes,
            by_kind: kinds
                .into_iter()
                .map(
                    |(kind, (object_count, encoded_bytes))| ArtifactReachabilityKindReportV1 {
                        kind,
                        object_count,
                        encoded_bytes,
                    },
                )
                .collect(),
            complete: true,
            recovery_commands: Vec::new(),
        })
    }

    /// Inspect one immutable artifact and all public lifecycle evidence bound to it.
    pub fn inspect_artifact(
        &self,
        envelope_id: &ArtifactEnvelopeId,
    ) -> Result<ArtifactInspectionReportV1> {
        let (desired_key, trust_scope, tree_root_id, object_id, state, verification_state) = self
            .conn
            .query_row(
                "SELECT desired_key,trust_scope,tree_root_id,object_id,state,verification_state
                 FROM artifact_envelopes WHERE envelope_id=?1",
                params![envelope_id.0],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, String>(5)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| Error::ObjectNotFound {
                kind: "artifact envelope",
                id: envelope_id.0.clone(),
            })?;
        let tree_root_id = ArtifactTreeId::parse(tree_root_id)
            .map_err(|error| Error::Corrupt(format!("invalid artifact tree ID: {error}")))?;
        let object_id = ObjectId(object_id);
        let envelope: ArtifactEnvelopeV1 = self.get_artifact_cas_object(
            &envelope_id.0,
            ARTIFACT_ENVELOPE_KIND,
            ARTIFACT_ENVELOPE_VERSION,
        )?;
        let backing_object_id =
            self.artifact_backing_object_id(&envelope_id.0, ARTIFACT_ENVELOPE_KIND)?;
        let (actual_envelope_id, _) = encode_artifact_envelope(envelope.clone())?;
        let encoded_desired_key = match &envelope.desired_identity {
            ArtifactDesiredIdentityV1::WorkspaceLayerV1 { cache_key, .. } => cache_key,
            ArtifactDesiredIdentityV1::ArtifactDesiredV2 { desired_key } => &desired_key.0,
        };
        if actual_envelope_id != *envelope_id
            || envelope.tree_root_id != tree_root_id
            || encoded_desired_key != &desired_key
            || envelope.trust_scope != trust_scope
            || backing_object_id != object_id.0
        {
            return Err(Error::Corrupt(format!(
                "artifact envelope `{envelope_id}` database identity disagrees with its object"
            )));
        }

        let bindings = self.artifact_generation_bindings(envelope_id)?;
        let attestations = self.artifact_attestations_for_envelope(envelope_id)?;
        let quarantines = self
            .list_artifact_quarantines()?
            .into_iter()
            .filter(|record| {
                record.incumbent_envelope_id.as_ref() == Some(envelope_id)
                    || record.candidate_envelope_id == *envelope_id
            })
            .collect::<Vec<_>>();
        let active_quarantine = quarantines.iter().find(|record| record.state == "active");
        let quarantine_state = if active_quarantine.is_some() {
            "active"
        } else if quarantines.is_empty() {
            "none"
        } else {
            "resolved"
        };
        let trust_state = if attestations.is_empty() {
            "missing"
        } else if attestations.iter().all(|report| {
            self.verify_artifact_attestation(&report.attestation_id)
                .is_ok_and(|verification| verification.valid)
        }) {
            "trusted"
        } else {
            "untrusted"
        };
        let recovery_commands = active_quarantine
            .map(|record| {
                vec![format!(
                    "trail env artifact quarantine show {}",
                    record.quarantine_id
                )]
            })
            .unwrap_or_default();
        let reachability = self.artifact_content_reachability(envelope_id)?;
        let storage =
            self.artifact_envelope_storage_accounting(envelope_id, &tree_root_id, &state)?;
        Ok(ArtifactInspectionReportV1 {
            envelope_id: envelope_id.clone(),
            object_id,
            desired_key,
            tree_root_id,
            state,
            verification_state,
            trust_state: trust_state.into(),
            quarantine_state: quarantine_state.into(),
            envelope,
            bindings,
            attestations,
            quarantines,
            reachability,
            storage,
            recovery_commands,
        })
    }

    /// Verify one artifact at an explicit evidence level. `reproduce` validates
    /// durable reproducibility evidence; executing a fresh producer remains a
    /// separate managed construction operation.
    pub fn verify_artifact(
        &self,
        envelope_id: &ArtifactEnvelopeId,
        level: ArtifactVerificationLevelV1,
    ) -> Result<ArtifactVerificationReportV1> {
        let inspection = self.inspect_artifact(envelope_id)?;
        let mut diagnostics = Vec::new();
        let content_identity_valid = true;
        let validation_receipts_valid = self
            .validate_envelope_validation_receipts(&inspection.envelope)
            .is_ok();
        if !validation_receipts_valid {
            diagnostics.push("artifact validation receipts are missing, failed, or stale".into());
        }
        let attestation_verifications = inspection
            .attestations
            .iter()
            .map(|attestation| self.verify_artifact_attestation(&attestation.attestation_id))
            .collect::<Result<Vec<_>>>()?;
        let attestations_valid = !attestation_verifications.is_empty()
            && attestation_verifications
                .iter()
                .all(|verification| verification.valid);
        if !attestations_valid {
            diagnostics.push("artifact attestation trust or binding verification failed".into());
        }

        let object_ids = self.artifact_envelope_object_ids(envelope_id)?;
        let tree_integrity = match level {
            ArtifactVerificationLevelV1::Attach => self
                .verified_artifact_tree_root(&inspection.tree_root_id)
                .map(|_| ()),
            ArtifactVerificationLevelV1::Sample => {
                let sorted = object_ids.iter().collect::<Vec<_>>();
                let mut sampled = BTreeSet::new();
                if let Some(first) = sorted.first() {
                    sampled.insert((*first).clone());
                }
                if let Some(middle) = sorted.get(sorted.len() / 2) {
                    sampled.insert((*middle).clone());
                }
                if let Some(last) = sorted.last() {
                    sampled.insert((*last).clone());
                }
                self.verify_artifact_object_set(&sampled)
            }
            ArtifactVerificationLevelV1::Full | ArtifactVerificationLevelV1::Reproduce => {
                self.verify_artifact_object_set(&object_ids).and_then(|_| {
                    self.artifact_tree_flat_entries(&inspection.tree_root_id)
                        .map(drop)
                })
            }
        };
        let tree_integrity_valid = tree_integrity.is_ok();
        if let Err(error) = tree_integrity {
            diagnostics.push(format!(
                "artifact tree integrity verification failed: {error}"
            ));
        }

        let reproduction_evidence_valid = if level == ArtifactVerificationLevelV1::Reproduce {
            let mut found = false;
            for receipt_id in &inspection.envelope.validation_receipt_ids {
                let receipt = self.artifact_validation_receipt(receipt_id)?;
                if receipt.declaration.kind == ArtifactValidationKindV1::Reproducibility
                    && receipt.outcome == ArtifactValidationOutcomeV1::Passed
                {
                    found = true;
                }
            }
            if !found {
                diagnostics.push(
                    "artifact has no passed reproducibility validation receipt; run a managed reproducibility construction before trusting this level"
                        .into(),
                );
            }
            Some(found)
        } else {
            None
        };

        if inspection.state != "ready" {
            diagnostics.push(format!(
                "artifact envelope state is `{}`, not `ready`",
                inspection.state
            ));
        }
        if inspection.verification_state != "verified" {
            diagnostics.push(format!(
                "artifact verification state is `{}`, not `verified`",
                inspection.verification_state
            ));
        }
        if inspection.quarantine_state == "active" {
            diagnostics.push("artifact desired identity is actively quarantined".into());
        }
        if inspection.trust_state != "trusted" {
            diagnostics.push(format!(
                "artifact producer trust state is `{}`",
                inspection.trust_state
            ));
        }
        let valid = inspection.state == "ready"
            && inspection.verification_state == "verified"
            && inspection.quarantine_state != "active"
            && inspection.trust_state == "trusted"
            && content_identity_valid
            && tree_integrity_valid
            && validation_receipts_valid
            && attestations_valid
            && reproduction_evidence_valid.unwrap_or(true);
        Ok(ArtifactVerificationReportV1 {
            envelope_id: envelope_id.clone(),
            level,
            desired_key: inspection.desired_key,
            tree_root_id: inspection.tree_root_id,
            envelope_state: inspection.state,
            verification_state: inspection.verification_state,
            trust_state: inspection.trust_state,
            quarantine_state: inspection.quarantine_state,
            content_identity_valid,
            tree_integrity_valid,
            validation_receipts_valid,
            attestations_valid,
            reproduction_evidence_valid,
            valid,
            diagnostics,
            recovery_commands: inspection.recovery_commands,
            reachability: inspection.reachability,
            storage: inspection.storage,
        })
    }

    fn artifact_envelope_tree_id(
        &self,
        envelope_id: &ArtifactEnvelopeId,
    ) -> Result<ArtifactTreeId> {
        let tree_root_id = self
            .conn
            .query_row(
                "SELECT tree_root_id FROM artifact_envelopes WHERE envelope_id=?1",
                params![envelope_id.0],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .ok_or_else(|| Error::ObjectNotFound {
                kind: "artifact envelope",
                id: envelope_id.0.clone(),
            })?;
        ArtifactTreeId::parse(tree_root_id)
            .map_err(|error| Error::Corrupt(format!("invalid artifact tree ID: {error}")))
    }

    fn artifact_generation_bindings(
        &self,
        envelope_id: &ArtifactEnvelopeId,
    ) -> Result<Vec<ArtifactGenerationBindingReportV1>> {
        let mut statement = self.conn.prepare(
            "SELECT binding_id,generation_id,component_id,output_name,desired_key,
                    envelope_id,tree_root_id,binding_identity,created_at
             FROM artifact_generation_bindings WHERE envelope_id=?1
             ORDER BY generation_id,component_id,output_name,binding_id",
        )?;
        let rows = statement
            .query_map(params![envelope_id.0], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if rows.len() > MAX_PUBLIC_ARTIFACT_REPORT_ITEMS {
            return Err(Error::InvalidInput(format!(
                "artifact inspection contains {} generation bindings; maximum is {MAX_PUBLIC_ARTIFACT_REPORT_ITEMS}",
                rows.len()
            )));
        }
        rows.into_iter()
            .map(
                |(
                    binding_id,
                    generation_id,
                    component_id,
                    output_name,
                    desired_key,
                    envelope_id,
                    tree_root_id,
                    binding_identity,
                    created_at,
                )| {
                    Ok(ArtifactGenerationBindingReportV1 {
                        binding_id,
                        generation_id,
                        component_id,
                        output_name,
                        desired_key,
                        envelope_id: ArtifactEnvelopeId::parse(envelope_id)
                            .map_err(Error::Corrupt)?,
                        tree_root_id: ArtifactTreeId::parse(tree_root_id)
                            .map_err(Error::Corrupt)?,
                        binding_identity,
                        created_at,
                    })
                },
            )
            .collect()
    }

    pub(crate) fn artifact_generation_bindings_for_generation(
        &self,
        generation_id: &str,
    ) -> Result<Vec<ArtifactGenerationBindingReportV1>> {
        let mut statement = self.conn.prepare(
            "SELECT binding_id,generation_id,component_id,output_name,desired_key,
                    envelope_id,tree_root_id,binding_identity,created_at
             FROM artifact_generation_bindings WHERE generation_id=?1
             ORDER BY component_id,output_name,binding_id",
        )?;
        let rows = statement
            .query_map(params![generation_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                    row.get::<_, i64>(8)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if rows.len() > MAX_PUBLIC_ARTIFACT_REPORT_ITEMS {
            return Err(Error::InvalidInput(format!(
                "environment generation `{generation_id}` contains {} artifact bindings; maximum is {MAX_PUBLIC_ARTIFACT_REPORT_ITEMS}",
                rows.len()
            )));
        }
        rows.into_iter()
            .map(
                |(
                    binding_id,
                    generation_id,
                    component_id,
                    output_name,
                    desired_key,
                    envelope_id,
                    tree_root_id,
                    binding_identity,
                    created_at,
                )| {
                    Ok(ArtifactGenerationBindingReportV1 {
                        binding_id,
                        generation_id,
                        component_id,
                        output_name,
                        desired_key,
                        envelope_id: ArtifactEnvelopeId::parse(envelope_id)
                            .map_err(Error::Corrupt)?,
                        tree_root_id: ArtifactTreeId::parse(tree_root_id)
                            .map_err(Error::Corrupt)?,
                        binding_identity,
                        created_at,
                    })
                },
            )
            .collect()
    }

    fn artifact_object_storage_rows(
        &self,
        object_ids: &BTreeSet<String>,
    ) -> Result<BTreeMap<String, (ObjectId, String, u64)>> {
        let mut objects = BTreeMap::new();
        let ids = object_ids.iter().collect::<Vec<_>>();
        for chunk in ids.chunks(512) {
            let placeholders = std::iter::repeat_n("?", chunk.len())
                .collect::<Vec<_>>()
                .join(",");
            let sql = format!(
                "SELECT object_id,kind,size_bytes FROM objects WHERE object_id IN ({placeholders})"
            );
            let mut statement = self.conn.prepare(&sql)?;
            let rows = statement.query_map(
                params_from_iter(chunk.iter().map(|object_id| object_id.as_str())),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                },
            )?;
            for row in rows {
                let (object_id, kind, size_bytes) = row?;
                let size_bytes = u64::try_from(size_bytes).map_err(|_| {
                    Error::Corrupt(format!("artifact object `{object_id}` has negative bytes"))
                })?;
                objects.insert(object_id.clone(), (ObjectId(object_id), kind, size_bytes));
            }
        }
        if objects.len() != object_ids.len() {
            let missing = object_ids
                .iter()
                .find(|object_id| !objects.contains_key(*object_id))
                .cloned()
                .unwrap_or_else(|| "unknown".into());
            return Err(Error::Corrupt(format!(
                "artifact reachability references missing object `{missing}`"
            )));
        }
        Ok(objects)
    }

    fn artifact_envelope_storage_accounting(
        &self,
        envelope_id: &ArtifactEnvelopeId,
        tree_root_id: &ArtifactTreeId,
        envelope_state: &str,
    ) -> Result<ArtifactStorageAccountingReport> {
        const MAX_INSPECTION_ENVELOPES: usize = 10_000;
        let selected = self.artifact_envelope_object_ids(envelope_id)?;
        let selected_rows = self.artifact_object_storage_rows(&selected)?;
        let envelope_ids = {
            let mut statement = self
                .conn
                .prepare("SELECT envelope_id FROM artifact_envelopes ORDER BY envelope_id")?;
            statement
                .query_map([], |row| row.get::<_, String>(0))?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        if envelope_ids.len() > MAX_INSPECTION_ENVELOPES {
            return Err(Error::InvalidInput(format!(
                "artifact inspection contains {} envelopes; maximum is {MAX_INSPECTION_ENVELOPES}",
                envelope_ids.len()
            )));
        }
        let mut reference_counts = BTreeMap::<String, u64>::new();
        let mut reference_count = 0_usize;
        for id in envelope_ids {
            let id = ArtifactEnvelopeId::parse(id).map_err(Error::Corrupt)?;
            let reachable = self.artifact_envelope_object_ids(&id)?;
            reference_count = reference_count
                .checked_add(reachable.len())
                .ok_or_else(|| {
                    Error::InvalidInput("artifact inspection reference count overflowed".into())
                })?;
            if reference_count > MAX_PUBLIC_ARTIFACT_OBJECT_REFERENCES {
                return Err(Error::InvalidInput(format!(
                    "artifact inspection contains more than {MAX_PUBLIC_ARTIFACT_OBJECT_REFERENCES} object references"
                )));
            }
            for object_id in reachable {
                *reference_counts.entry(object_id).or_default() += 1;
            }
        }
        let mut unique_authoritative_bytes = 0_u64;
        let mut cross_artifact_shared_bytes = 0_u64;
        for (object_id, (_, _, bytes)) in &selected_rows {
            if reference_counts.get(object_id).copied().unwrap_or(1) > 1 {
                cross_artifact_shared_bytes = cross_artifact_shared_bytes.saturating_add(*bytes);
            } else {
                unique_authoritative_bytes = unique_authoritative_bytes.saturating_add(*bytes);
            }
        }
        let tree: ArtifactTreeRootV1 = self.get_artifact_cas_object(
            &tree_root_id.0,
            ARTIFACT_TREE_ROOT_KIND,
            ARTIFACT_TREE_ROOT_VERSION,
        )?;
        let materialized = self.conn.query_row(
            "SELECT COALESCE(SUM(COALESCE(physical_bytes,0)),0)
             FROM artifact_materializations WHERE tree_root_id=?1",
            params![tree_root_id.0],
            |row| row.get::<_, i64>(0),
        )?;
        let layer_materialized = self.conn.query_row(
            "SELECT COALESCE(SUM(COALESCE(w.physical_bytes,0)),0)
             FROM workspace_layer_artifact_shadows s
             JOIN workspace_layers w ON w.layer_id=s.layer_id
             WHERE s.envelope_id=?1",
            params![envelope_id.0],
            |row| row.get::<_, i64>(0),
        )?;
        let materialized_bytes = u64::try_from(materialized)
            .and_then(|left| {
                u64::try_from(layer_materialized).map(|right| left.saturating_add(right))
            })
            .map_err(|_| Error::Corrupt("artifact materialization has negative bytes".into()))?;
        Ok(ArtifactStorageAccountingReport {
            logical_bytes: tree.logical_bytes,
            unique_authoritative_bytes,
            cross_artifact_shared_bytes,
            materialized_bytes,
            lane_private_bytes: 0,
            prefetched_bytes: 0,
            demand_loaded_bytes: 0,
            reclaimable_bytes: 0,
            unknown_bytes: 0,
            accounting: format!(
                "scope=artifact;state={envelope_state};axes=logical|authoritative|physical|disposition;authoritative=encoded-cbor-bytes-deduplicated;lane-private=not-applicable;reclaimable=requires-workspace-gc-analysis"
            ),
        })
    }

    fn verify_artifact_object_set(&self, object_ids: &BTreeSet<String>) -> Result<()> {
        for object_id in object_ids {
            let (kind, version, codec, hash_alg, size_bytes, bytes, artifact_id, logical_bytes) =
                self.conn
                    .query_row(
                        "SELECT o.kind,o.version,o.codec,o.hash_alg,o.size_bytes,o.bytes,
                            a.artifact_id,a.logical_bytes
                     FROM objects o LEFT JOIN artifact_objects a ON a.object_id=o.object_id
                     WHERE o.object_id=?1",
                        params![object_id],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, i64>(1)?,
                                row.get::<_, String>(2)?,
                                row.get::<_, String>(3)?,
                                row.get::<_, i64>(4)?,
                                row.get::<_, Vec<u8>>(5)?,
                                row.get::<_, Option<String>>(6)?,
                                row.get::<_, Option<i64>>(7)?,
                            ))
                        },
                    )
                    .optional()?
                    .ok_or_else(|| Error::ObjectNotFound {
                        kind: "artifact reachable object",
                        id: object_id.clone(),
                    })?;
            let version_u16 = u16::try_from(version).map_err(|_| {
                Error::Corrupt(format!("artifact object `{object_id}` has invalid version"))
            })?;
            if codec != "cbor"
                || hash_alg != "sha256"
                || size_bytes != i64::try_from(bytes.len()).unwrap_or(-1)
                || ObjectId::for_bytes(&kind, version_u16, &bytes).0 != *object_id
            {
                return Err(Error::Corrupt(format!(
                    "artifact reachable object `{object_id}` failed content identity verification"
                )));
            }
            if let (Some(artifact_id), Some(logical_bytes)) = (artifact_id, logical_bytes) {
                self.validate_artifact_cas_object(
                    &artifact_id,
                    &kind,
                    version,
                    logical_bytes,
                    &bytes,
                )?;
            }
        }
        Ok(())
    }

    fn artifact_materialized_bytes(
        &self,
        view_id: Option<&str>,
        selected_trees: &BTreeSet<String>,
    ) -> Result<u64> {
        let materializations = if selected_trees.is_empty() {
            0
        } else {
            let tree_ids = selected_trees.iter().collect::<Vec<_>>();
            let mut total = 0_u64;
            for chunk in tree_ids.chunks(512) {
                let placeholders = std::iter::repeat_n("?", chunk.len())
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT COALESCE(SUM(COALESCE(physical_bytes,0)),0)
                     FROM artifact_materializations WHERE tree_root_id IN ({placeholders})"
                );
                let bytes = self.conn.query_row(
                    &sql,
                    params_from_iter(chunk.iter().map(|tree_id| tree_id.as_str())),
                    |row| row.get::<_, i64>(0),
                )?;
                total = total.saturating_add(u64::try_from(bytes).map_err(|_| {
                    Error::Corrupt("artifact materialization has negative physical bytes".into())
                })?);
            }
            total
        };
        let layer_bytes = if let Some(view_id) = view_id {
            self.conn.query_row(
                "SELECT COALESCE(SUM(COALESCE(w.physical_bytes,0)),0)
                 FROM workspace_view_layers l
                 JOIN workspace_layers w ON w.layer_id=l.layer_id
                 WHERE l.view_id=?1
                   AND EXISTS (
                       SELECT 1 FROM workspace_layer_artifact_shadows s
                       WHERE s.layer_id=w.layer_id
                   )",
                params![view_id],
                |row| row.get::<_, i64>(0),
            )?
        } else {
            self.conn.query_row(
                "SELECT COALESCE(SUM(COALESCE(w.physical_bytes,0)),0)
                 FROM workspace_layers w
                 WHERE EXISTS (
                     SELECT 1 FROM workspace_layer_artifact_shadows s
                     WHERE s.layer_id=w.layer_id
                 )",
                [],
                |row| row.get::<_, i64>(0),
            )?
        };
        Ok(materializations.saturating_add(
            u64::try_from(layer_bytes).map_err(|_| {
                Error::Corrupt("workspace layer has negative physical bytes".into())
            })?,
        ))
    }

    pub(crate) fn artifact_tree_lazy_entry(
        &self,
        tree_id: &ArtifactTreeId,
        relative_path: &str,
    ) -> Result<Option<ArtifactLazyEntry>> {
        let tree = self.verified_artifact_tree_root(tree_id)?;
        if relative_path.is_empty() {
            return Ok(Some(ArtifactLazyEntry::Directory {
                node_id: tree.root_directory_id,
            }));
        }
        let relative_path = normalize_relative_path(relative_path)?;
        let mut directory_id = tree.root_directory_id;
        let mut segments = relative_path.split('/').peekable();
        while let Some(segment) = segments.next() {
            let directory = self.verified_artifact_directory(&directory_id)?;
            let Ok(index) = directory
                .entries
                .binary_search_by(|entry| entry.name.as_str().cmp(segment))
            else {
                return Ok(None);
            };
            let target = directory.entries[index].target.clone();
            if segments.peek().is_some() {
                match target {
                    ArtifactDirectoryEntryTargetV1::Directory { node_id } => {
                        directory_id = node_id;
                    }
                    ArtifactDirectoryEntryTargetV1::File { .. }
                    | ArtifactDirectoryEntryTargetV1::Symlink { .. } => return Ok(None),
                }
            } else {
                return self.artifact_lazy_entry_from_target(target).map(Some);
            }
        }
        Ok(None)
    }

    pub(crate) fn artifact_tree_lazy_children(
        &self,
        tree_id: &ArtifactTreeId,
        relative_path: &str,
    ) -> Result<Vec<(String, ArtifactLazyEntry)>> {
        let Some(ArtifactLazyEntry::Directory { node_id }) =
            self.artifact_tree_lazy_entry(tree_id, relative_path)?
        else {
            return Ok(Vec::new());
        };
        let directory = self.verified_artifact_directory(&node_id)?;
        directory
            .entries
            .into_iter()
            .map(|entry| {
                Ok((
                    entry.name,
                    self.artifact_lazy_entry_from_target(entry.target)?,
                ))
            })
            .collect()
    }

    #[cfg(windows)]
    pub(crate) fn artifact_tree_lazy_follow_symlink(
        &self,
        tree_id: &ArtifactTreeId,
        link_path: &str,
        target: &str,
    ) -> Result<Option<ArtifactLazyEntry>> {
        let mut path = resolve_artifact_symlink_path(link_path, target)?;
        for _ in 0..40 {
            let Some(entry) = self.artifact_tree_lazy_entry(tree_id, &path)? else {
                return Ok(None);
            };
            match entry {
                ArtifactLazyEntry::Symlink { target } => {
                    path = resolve_artifact_symlink_path(&path, &target)?;
                }
                entry => return Ok(Some(entry)),
            }
        }
        Err(Error::Corrupt(format!(
            "artifact symlink `{link_path}` exceeds the resolution bound"
        )))
    }

    pub(crate) fn artifact_file_read_range(
        &self,
        file_id: &ArtifactFileId,
        offset: u64,
        count: u32,
    ) -> Result<Vec<u8>> {
        let file = self.verified_artifact_file(file_id)?;
        if offset >= file.size_bytes || count == 0 {
            return Ok(Vec::new());
        }
        let end = offset.saturating_add(u64::from(count)).min(file.size_bytes);
        match file.content {
            ArtifactFileContentV1::Blob { blob_id } => {
                let blob: ArtifactBlobV1 = self.get_artifact_cas_object(
                    &blob_id.0,
                    ARTIFACT_BLOB_KIND,
                    ARTIFACT_BLOB_VERSION,
                )?;
                let (actual, _) = encode_artifact_blob(blob.clone())?;
                if actual != blob_id || blob.bytes.len() as u64 != file.size_bytes {
                    return Err(Error::Corrupt(
                        "artifact blob identity or file-size edge is invalid".into(),
                    ));
                }
                Ok(blob.bytes[offset as usize..end as usize].to_vec())
            }
            ArtifactFileContentV1::Chunks { chunk_list_id } => {
                let list: ArtifactChunkListV1 = self.get_artifact_cas_object(
                    &chunk_list_id.0,
                    ARTIFACT_CHUNK_LIST_KIND,
                    ARTIFACT_CHUNK_LIST_VERSION,
                )?;
                let (actual, _) = encode_artifact_chunk_list(list.clone())?;
                if actual != chunk_list_id
                    || list.file_size_bytes != file.size_bytes
                    || list.file_sha256 != file.content_sha256
                {
                    return Err(Error::Corrupt(
                        "artifact chunk-list identity or file edge is invalid".into(),
                    ));
                }
                let mut output = Vec::with_capacity((end - offset) as usize);
                let mut chunk_start = 0u64;
                for chunk_ref in list.chunks {
                    let chunk_end = chunk_start
                        .checked_add(chunk_ref.size_bytes)
                        .ok_or_else(|| Error::Corrupt("artifact chunk range overflow".into()))?;
                    if chunk_end > offset && chunk_start < end {
                        let chunk: ArtifactChunkV1 = self.get_artifact_cas_object(
                            &chunk_ref.chunk_id.0,
                            ARTIFACT_CHUNK_KIND,
                            ARTIFACT_CHUNK_VERSION,
                        )?;
                        let (actual, _) = encode_artifact_chunk(chunk.clone())?;
                        if actual != chunk_ref.chunk_id
                            || chunk.bytes.len() as u64 != chunk_ref.size_bytes
                        {
                            return Err(Error::Corrupt(
                                "artifact chunk identity or size edge is invalid".into(),
                            ));
                        }
                        let selected_start = offset.saturating_sub(chunk_start) as usize;
                        let selected_end = end.min(chunk_end).saturating_sub(chunk_start) as usize;
                        output.extend_from_slice(&chunk.bytes[selected_start..selected_end]);
                    }
                    chunk_start = chunk_end;
                    if chunk_start >= end {
                        break;
                    }
                }
                if output.len() as u64 != end - offset {
                    return Err(Error::Corrupt(
                        "artifact ranged read did not cover the requested file extent".into(),
                    ));
                }
                Ok(output)
            }
        }
    }

    pub(crate) fn materialize_artifact_file(
        &self,
        file_id: &ArtifactFileId,
        destination: &Path,
    ) -> Result<u32> {
        let file = self.verified_artifact_file(file_id)?;
        self.verify_artifact_file_content(&file)?;
        // Open outside the cleanup scope so a create-new collision never
        // removes a destination that this materialization attempt did not
        // create.
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)?;
        let materialized = (|| -> Result<()> {
            // Stream bounded ranges so copy-up never allocates a complete
            // large artifact file. The complete digest was verified before
            // publication.
            let mut offset = 0u64;
            while offset < file.size_bytes {
                let part = self.artifact_file_read_range(file_id, offset, 4 * 1024 * 1024)?;
                if part.is_empty() {
                    return Err(Error::Corrupt(
                        "artifact file materialization made no progress".into(),
                    ));
                }
                output.write_all(&part)?;
                offset += part.len() as u64;
            }
            output.sync_all()?;
            set_artifact_materialized_mode(destination, file.mode)?;
            Ok(())
        })();
        if let Err(error) = materialized {
            let _ = fs::remove_file(destination);
            return Err(error);
        }
        Ok(file.mode)
    }

    pub(crate) fn ensure_artifact_tree_materialization(
        &self,
        tree_id: &ArtifactTreeId,
    ) -> Result<ArtifactMaterializationReport> {
        let _lock = self.acquire_write_lock()?;
        self.ensure_artifact_tree_materialization_under_write_lock(tree_id)
    }

    pub(crate) fn ensure_artifact_tree_materialization_under_write_lock(
        &self,
        tree_id: &ArtifactTreeId,
    ) -> Result<ArtifactMaterializationReport> {
        let tree = self.verified_artifact_tree_root(tree_id)?;
        let backend_compatibility = artifact_materialization_backend_compatibility();
        let identity_seed = format!("{}\0{backend_compatibility}", tree_id.0);
        let materialization_id = format!(
            "materialization_{}",
            crate::ids::short_hash(identity_seed.as_bytes(), 32)
        );
        let (materialization_parent, staging_parent) =
            self.artifact_materialization_cache_parents()?;
        let final_path = materialization_parent.join(&materialization_id);
        let final_exists = real_artifact_materialization_directory_exists(
            &final_path,
            "artifact materialization",
        )?;
        let existing = self
            .conn
            .query_row(
                "SELECT materialization_id,storage_path,state,logical_bytes,
                        COALESCE(physical_bytes,0),entry_count
                 FROM artifact_materializations
                 WHERE tree_root_id=?1 AND backend_compatibility=?2",
                params![tree_id.0, &backend_compatibility],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, i64>(4)?,
                        row.get::<_, i64>(5)?,
                    ))
                },
            )
            .optional()?;
        if let Some((stored_id, stored_path, state, logical, physical, entries)) = existing {
            if stored_id != materialization_id || Path::new(&stored_path) != final_path {
                return Err(Error::Corrupt(format!(
                    "artifact materialization `{stored_id}` has a non-canonical identity or storage path"
                )));
            }
            if state == "verified" && final_exists {
                match self.verify_artifact_materialization(tree_id, &tree, &final_path) {
                    Ok(()) => {
                        self.conn.execute(
                            "UPDATE artifact_materializations SET last_used_at=?1
                             WHERE materialization_id=?2 AND state='verified'",
                            params![now_ts(), &materialization_id],
                        )?;
                        return Ok(ArtifactMaterializationReport {
                            materialization_id,
                            tree_root_id: tree_id.clone(),
                            backend_compatibility,
                            storage_path: final_path,
                            logical_bytes: u64::try_from(logical).map_err(|_| {
                                Error::Corrupt(
                                    "artifact materialization has negative logical bytes".into(),
                                )
                            })?,
                            physical_bytes: u64::try_from(physical).map_err(|_| {
                                Error::Corrupt(
                                    "artifact materialization has negative physical bytes".into(),
                                )
                            })?,
                            entry_count: u64::try_from(entries).map_err(|_| {
                                Error::Corrupt(
                                    "artifact materialization has negative entry count".into(),
                                )
                            })?,
                            reused: true,
                        });
                    }
                    Err(_) => {
                        super::workspace_layer::make_tree_writable(&final_path);
                        fs::remove_dir_all(&final_path)?;
                    }
                }
            } else if final_exists {
                super::workspace_layer::make_tree_writable(&final_path);
                fs::remove_dir_all(&final_path)?;
            }
            self.conn.execute(
                "UPDATE artifact_materializations SET state='failed',last_used_at=?1
                 WHERE materialization_id=?2",
                params![now_ts(), &materialization_id],
            )?;
        } else if final_exists {
            match self.verify_artifact_materialization(tree_id, &tree, &final_path) {
                Ok(()) => {
                    let physical = super::workspace_layer::layer_physical_bytes(&final_path)?;
                    self.upsert_verified_artifact_materialization(
                        &materialization_id,
                        tree_id,
                        &backend_compatibility,
                        &final_path,
                        &tree,
                        physical,
                    )?;
                    return Ok(ArtifactMaterializationReport {
                        materialization_id,
                        tree_root_id: tree_id.clone(),
                        backend_compatibility,
                        storage_path: final_path,
                        logical_bytes: tree.logical_bytes,
                        physical_bytes: physical,
                        entry_count: tree.entry_count,
                        reused: true,
                    });
                }
                Err(_) => {
                    super::workspace_layer::make_tree_writable(&final_path);
                    fs::remove_dir_all(&final_path)?;
                }
            }
        }

        let staging = staging_parent.join(format!("artifact_{materialization_id}"));
        if real_artifact_materialization_directory_exists(
            &staging,
            "artifact materialization staging",
        )? {
            super::workspace_layer::make_tree_writable(&staging);
            fs::remove_dir_all(&staging)?;
        }
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO artifact_materializations(
                materialization_id,tree_root_id,backend_compatibility,storage_path,state,
                logical_bytes,physical_bytes,entry_count,last_used_at,created_at
             ) VALUES(?1,?2,?3,?4,'building',?5,NULL,?6,?7,?7)
             ON CONFLICT(tree_root_id,backend_compatibility) DO UPDATE SET
                materialization_id=excluded.materialization_id,
                storage_path=excluded.storage_path,state='building',
                logical_bytes=excluded.logical_bytes,physical_bytes=NULL,
                entry_count=excluded.entry_count,last_used_at=excluded.last_used_at",
            params![
                &materialization_id,
                tree_id.0,
                &backend_compatibility,
                final_path.to_string_lossy(),
                i64::try_from(tree.logical_bytes).map_err(|_| Error::InvalidInput(
                    "artifact materialization logical bytes exceed SQLite range".into()
                ))?,
                i64::try_from(tree.entry_count).map_err(|_| Error::InvalidInput(
                    "artifact materialization entry count exceeds SQLite range".into()
                ))?,
                now,
            ],
        )?;
        let publication = (|| -> Result<u64> {
            self.materialize_artifact_tree_under_write_lock(tree_id, &staging)
                .map_err(|error| {
                    Error::Corrupt(format!(
                        "artifact materialization content projection failed: {error}"
                    ))
                })?;
            let entries =
                super::workspace_layer::scan_layer_entries(&staging, true).map_err(|error| {
                    Error::Corrupt(format!(
                        "artifact materialization immutable scan failed: {error}"
                    ))
                })?;
            super::workspace_layer::verify_artifact_shadow_matches_layer_entries(
                &tree,
                &self.artifact_tree_flat_entries(tree_id)?,
                &entries,
            )?;
            let physical =
                super::workspace_layer::layer_physical_bytes(&staging).map_err(|error| {
                    Error::Corrupt(format!(
                        "artifact materialization physical accounting failed: {error}"
                    ))
                })?;
            fs::rename(&staging, &final_path).map_err(|error| {
                Error::Corrupt(format!(
                    "artifact materialization atomic publication failed: {error}"
                ))
            })?;
            // macOS rejects renaming a read-only directory even when both
            // parents are writable. Child entries are already immutable, so
            // publish the root while the workspace write lock is held and
            // seal that root before marking the database row verified.
            super::workspace_layer::set_layer_read_only(&final_path, true, 0o755).map_err(
                |error| {
                    Error::Corrupt(format!(
                        "artifact materialization root sealing failed: {error}"
                    ))
                },
            )?;
            sync_directory(final_path.parent().unwrap());
            Ok(physical)
        })();
        let physical = match publication {
            Ok(physical) => physical,
            Err(error) => {
                super::workspace_layer::make_tree_writable(&staging);
                let _ = fs::remove_dir_all(&staging);
                super::workspace_layer::make_tree_writable(&final_path);
                let _ = fs::remove_dir_all(&final_path);
                self.conn.execute(
                    "UPDATE artifact_materializations SET state='failed',last_used_at=?1
                     WHERE materialization_id=?2",
                    params![now_ts(), &materialization_id],
                )?;
                return Err(error);
            }
        };
        self.upsert_verified_artifact_materialization(
            &materialization_id,
            tree_id,
            &backend_compatibility,
            &final_path,
            &tree,
            physical,
        )?;
        Ok(ArtifactMaterializationReport {
            materialization_id,
            tree_root_id: tree_id.clone(),
            backend_compatibility,
            storage_path: final_path,
            logical_bytes: tree.logical_bytes,
            physical_bytes: physical,
            entry_count: tree.entry_count,
            reused: false,
        })
    }

    fn artifact_materialization_cache_parents(&self) -> Result<(PathBuf, PathBuf)> {
        // Reuse the environment executor's descriptor-validated staging
        // hierarchy rather than independently trusting `.trail/cache` path
        // components.
        let staging = self.workspace_environment_staging_parent()?;
        let cache = staging.parent().ok_or_else(|| {
            Error::Corrupt("artifact materialization staging has no cache parent".into())
        })?;
        let materializations = cache.join("artifact-materializations");
        match fs::symlink_metadata(&materializations) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(Error::InvalidPath {
                    path: materializations.to_string_lossy().into_owned(),
                    reason: "artifact materialization cache must remain a real directory inside Trail storage"
                        .into(),
                });
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&materializations)?;
            }
            Err(error) => return Err(Error::Io(error)),
        }
        let canonical = fs::canonicalize(&materializations)?;
        if canonical != materializations {
            return Err(Error::InvalidPath {
                path: materializations.to_string_lossy().into_owned(),
                reason: "artifact materialization cache escaped Trail storage".into(),
            });
        }
        Ok((canonical, staging))
    }

    fn verify_artifact_materialization(
        &self,
        tree_id: &ArtifactTreeId,
        tree: &ArtifactTreeRootV1,
        path: &Path,
    ) -> Result<()> {
        // Verification also restores immutable permissions. Content identity
        // alone is insufficient because a writable cache could become a
        // mutable alias of authoritative CAS content after this check.
        let entries = super::workspace_layer::scan_layer_entries(path, true)?;
        super::workspace_layer::verify_artifact_shadow_matches_layer_entries(
            tree,
            &self.artifact_tree_flat_entries(tree_id)?,
            &entries,
        )?;
        super::workspace_layer::set_layer_read_only(path, true, 0o755)
    }

    fn upsert_verified_artifact_materialization(
        &self,
        materialization_id: &str,
        tree_id: &ArtifactTreeId,
        backend_compatibility: &str,
        path: &Path,
        tree: &ArtifactTreeRootV1,
        physical_bytes: u64,
    ) -> Result<()> {
        let now = now_ts();
        self.conn.execute(
            "INSERT INTO artifact_materializations(
                materialization_id,tree_root_id,backend_compatibility,storage_path,state,
                logical_bytes,physical_bytes,entry_count,last_used_at,created_at
             ) VALUES(?1,?2,?3,?4,'verified',?5,?6,?7,?8,?8)
             ON CONFLICT(tree_root_id,backend_compatibility) DO UPDATE SET
                materialization_id=excluded.materialization_id,
                storage_path=excluded.storage_path,state='verified',
                logical_bytes=excluded.logical_bytes,physical_bytes=excluded.physical_bytes,
                entry_count=excluded.entry_count,last_used_at=excluded.last_used_at",
            params![
                materialization_id,
                tree_id.0,
                backend_compatibility,
                path.to_string_lossy(),
                i64::try_from(tree.logical_bytes).map_err(|_| Error::InvalidInput(
                    "artifact materialization logical bytes exceed SQLite range".into()
                ))?,
                i64::try_from(physical_bytes).map_err(|_| Error::InvalidInput(
                    "artifact materialization physical bytes exceed SQLite range".into()
                ))?,
                i64::try_from(tree.entry_count).map_err(|_| Error::InvalidInput(
                    "artifact materialization entry count exceeds SQLite range".into()
                ))?,
                now,
            ],
        )?;
        Ok(())
    }

    fn verified_artifact_tree_root(&self, tree_id: &ArtifactTreeId) -> Result<ArtifactTreeRootV1> {
        let tree: ArtifactTreeRootV1 = self.get_artifact_cas_object(
            &tree_id.0,
            ARTIFACT_TREE_ROOT_KIND,
            ARTIFACT_TREE_ROOT_VERSION,
        )?;
        let (actual, _) = encode_artifact_tree_root(tree.clone())?;
        if actual != *tree_id {
            return Err(Error::Corrupt(format!(
                "artifact tree root `{tree_id}` has conflicting encoded identity"
            )));
        }
        Ok(tree)
    }

    fn verified_artifact_directory(
        &self,
        directory_id: &ArtifactTreeId,
    ) -> Result<ArtifactDirectoryNodeV1> {
        let directory: ArtifactDirectoryNodeV1 = self.get_artifact_cas_object(
            &directory_id.0,
            ARTIFACT_DIRECTORY_NODE_KIND,
            ARTIFACT_DIRECTORY_NODE_VERSION,
        )?;
        let (actual, canonical) = encode_artifact_directory_node(directory.clone())?;
        if actual != *directory_id || from_cbor::<ArtifactDirectoryNodeV1>(&canonical)? != directory
        {
            return Err(Error::Corrupt(format!(
                "artifact directory `{directory_id}` has conflicting encoded identity"
            )));
        }
        Ok(directory)
    }

    fn verified_artifact_file(&self, file_id: &ArtifactFileId) -> Result<ArtifactFileNodeV1> {
        let file: ArtifactFileNodeV1 = self.get_artifact_cas_object(
            &file_id.0,
            ARTIFACT_FILE_NODE_KIND,
            ARTIFACT_FILE_NODE_VERSION,
        )?;
        let (actual, _) = encode_artifact_file_node(file.clone())?;
        if actual != *file_id {
            return Err(Error::Corrupt(format!(
                "artifact file `{file_id}` has conflicting encoded identity"
            )));
        }
        Ok(file)
    }

    fn artifact_lazy_entry_from_target(
        &self,
        target: ArtifactDirectoryEntryTargetV1,
    ) -> Result<ArtifactLazyEntry> {
        match target {
            ArtifactDirectoryEntryTargetV1::Directory { node_id } => {
                Ok(ArtifactLazyEntry::Directory { node_id })
            }
            ArtifactDirectoryEntryTargetV1::File { node_id } => {
                let file = self.verified_artifact_file(&node_id)?;
                Ok(ArtifactLazyEntry::File {
                    node_id,
                    mode: file.mode,
                    size_bytes: file.size_bytes,
                })
            }
            ArtifactDirectoryEntryTargetV1::Symlink { target } => {
                validate_artifact_symlink_target(&target)?;
                Ok(ArtifactLazyEntry::Symlink { target })
            }
        }
    }

    pub(crate) fn put_legacy_artifact_envelope_under_write_lock(
        &self,
        layer_key: &WorkspaceLayerKeyV1,
        cache_key: &str,
        tree_root_id: ArtifactTreeId,
    ) -> Result<ArtifactEnvelopeId> {
        validate_resolution_text(cache_key, "legacy layer cache key")?;
        let canonical_cache_key = self.workspace_layer_cache_key(layer_key)?;
        if canonical_cache_key != cache_key {
            return Err(Error::InvalidInput(
                "workspace layer key changed before artifact sealing".into(),
            ));
        }
        let tree = self.verified_artifact_tree_root(&tree_root_id)?;
        let entries = self.artifact_tree_flat_entries(&tree_root_id)?;
        if entries.len() as u64 != tree.entry_count
            || entries
                .values()
                .try_fold(0u64, |total, entry| total.checked_add(entry.size_bytes))
                != Some(tree.logical_bytes)
        {
            return Err(Error::Corrupt(
                "workspace layer artifact tree is not a complete content identity".into(),
            ));
        }
        let desired_identity = ArtifactDesiredIdentityV1::WorkspaceLayerV1 {
            cache_key: cache_key.to_string(),
            canonical_key: layer_key.clone(),
        };
        let validation_receipt_ids = [
            self.put_host_workspace_layer_seal_receipt_under_write_lock(
                ArtifactValidationV1 {
                    name: HOST_WORKSPACE_LAYER_STRUCTURAL_SEAL.into(),
                    kind: ArtifactValidationKindV1::Structural,
                    required: true,
                    parameters: BTreeMap::from([
                        ("content_identity".into(), "artifact-tree-v1".into()),
                        ("path_normalizer".into(), tree.path_normalizer.clone()),
                    ]),
                },
                desired_identity.clone(),
                tree_root_id.clone(),
                BTreeMap::from([
                    ("complete_tree_identity".into(), "passed".into()),
                    ("declared_path_containment".into(), "passed".into()),
                    ("entry_count".into(), tree.entry_count.to_string()),
                    ("limits".into(), "passed".into()),
                    ("logical_bytes".into(), tree.logical_bytes.to_string()),
                    ("safe_normalized_content".into(), "passed".into()),
                    ("secret_policy".into(), "passed".into()),
                ]),
            )?,
            self.put_host_workspace_layer_seal_receipt_under_write_lock(
                ArtifactValidationV1 {
                    name: HOST_WORKSPACE_LAYER_POLICY_SEAL.into(),
                    kind: ArtifactValidationKindV1::Policy,
                    required: true,
                    parameters: BTreeMap::from([
                        ("pin_contract".into(), "workspace-layer-key-v1".into()),
                        ("trust_scope".into(), "workspace-layer-v1".into()),
                    ]),
                },
                desired_identity.clone(),
                tree_root_id.clone(),
                BTreeMap::from([
                    ("desired_pins".into(), "unchanged".into()),
                    (
                        "producer_termination".into(),
                        "terminated_or_disconnected".into(),
                    ),
                    ("producer_trust".into(), "local_host_authorized".into()),
                ]),
            )?,
        ]
        .into_iter()
        .collect();
        let envelope = ArtifactEnvelopeV1 {
            version: ARTIFACT_ENVELOPE_VERSION,
            desired_identity,
            tree_root_id,
            component_id: format!("legacy:{}", layer_key.adapter),
            output_name: "legacy-layer".into(),
            output_policy: EnvironmentOutputPolicy::ImmutableSeedPrivate,
            portability_scope: layer_key.portability_scope.clone(),
            trust_scope: "workspace-layer-v1".into(),
            secret_taint: ArtifactSecretTaintV1::Clear,
            resolution_snapshot_id: None,
            validation_receipt_ids,
        };
        let (envelope_id, quarantined) = self.put_artifact_envelope_under_write_lock(envelope)?;
        if quarantined {
            return Err(Error::InvalidInput(format!(
                "artifact desired identity `{cache_key}` produced divergent content and was quarantined"
            )));
        }
        Ok(envelope_id)
    }

    /// Replace a workspace layer's legacy CAS shadow with a verified desired-key-v2
    /// envelope over the exact same immutable tree. The physical layer remains a
    /// compatibility materialization; the envelope becomes artifact authority for
    /// activation, inheritance, export, reachability, and collection.
    pub(crate) fn bind_workspace_layer_artifact_v2(
        &self,
        layer_id: &str,
        envelope: ArtifactEnvelopeV1,
    ) -> Result<ArtifactEnvelopeId> {
        let ArtifactDesiredIdentityV1::ArtifactDesiredV2 { .. } = &envelope.desired_identity else {
            return Err(Error::InvalidInput(
                "workspace artifact-v2 binding requires a desired-key-v2 envelope".into(),
            ));
        };
        if !envelope.output_policy.has_immutable_layer() {
            return Err(Error::InvalidInput(
                "workspace artifact-v2 binding requires an immutable output policy".into(),
            ));
        }
        let _lock = self.acquire_write_lock()?;
        let layer = self.verify_workspace_layer_for_attach(layer_id)?;
        let (tree_root_id, state) = self.conn.query_row(
            "SELECT tree_root_id,state FROM workspace_layer_artifact_shadows WHERE layer_id=?1",
            params![layer_id],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )?;
        if state != "verified" || tree_root_id != envelope.tree_root_id.0 {
            return Err(Error::InvalidInput(format!(
                "workspace layer `{layer_id}` does not contain the exact artifact-v2 tree"
            )));
        }
        if layer.portability_scope != envelope.portability_scope {
            return Err(Error::InvalidInput(format!(
                "workspace layer `{layer_id}` and artifact-v2 envelope have different portability scopes"
            )));
        }
        let tree_root_id = ArtifactTreeId::parse(tree_root_id)
            .map_err(|error| Error::Corrupt(format!("invalid artifact tree ID: {error}")))?;
        self.artifact_tree_flat_entries(&tree_root_id)?;
        let (envelope_id, quarantined) = self.put_artifact_envelope_under_write_lock(envelope)?;
        if quarantined {
            return Err(Error::InvalidInput(format!(
                "workspace layer `{layer_id}` produced divergent artifact-v2 content and was quarantined"
            )));
        }
        let updated = self.conn.execute(
            "UPDATE workspace_layer_artifact_shadows
             SET envelope_id=?1,state='verified',verified_at=?2
             WHERE layer_id=?3 AND tree_root_id=?4 AND state='verified'",
            params![envelope_id.0, now_ts(), layer_id, tree_root_id.0],
        )?;
        if updated != 1 {
            return Err(Error::Corrupt(format!(
                "workspace layer `{layer_id}` artifact binding changed while publishing v2 authority"
            )));
        }
        Ok(envelope_id)
    }

    pub(crate) fn put_artifact_envelope_under_write_lock(
        &self,
        mut envelope: ArtifactEnvelopeV1,
    ) -> Result<(ArtifactEnvelopeId, bool)> {
        envelope.validation_receipt_ids.sort();
        envelope.validation_receipt_ids.dedup();
        let desired_key = match &envelope.desired_identity {
            ArtifactDesiredIdentityV1::WorkspaceLayerV1 { cache_key, .. } => cache_key.clone(),
            ArtifactDesiredIdentityV1::ArtifactDesiredV2 { desired_key } => desired_key.0.clone(),
        };
        validate_resolution_text(&desired_key, "artifact desired key")?;
        validate_resolution_text(&envelope.trust_scope, "artifact trust scope")?;
        validate_artifact_secret_taint(&envelope.secret_taint)?;
        if !envelope.secret_taint.is_clear() {
            return Err(Error::InvalidInput(
                "secret-tainted artifact output must remain lane-private and cannot enter shared CAS"
                    .into(),
            ));
        }
        self.validate_envelope_validation_receipts(&envelope)?;
        let (envelope_id, _) = encode_artifact_envelope(envelope.clone())?;
        let object_id = self.put_artifact_cas_object(
            &envelope_id.0,
            ARTIFACT_ENVELOPE_KIND,
            ARTIFACT_ENVELOPE_VERSION,
            0,
            &envelope,
        )?;
        self.conn
            .execute_batch("SAVEPOINT trail_artifact_envelope")?;
        let publication = (|| -> Result<bool> {
            let incumbent = self
                .conn
                .query_row(
                    "SELECT envelope_id,tree_root_id FROM artifact_envelopes
                     WHERE desired_key=?1 AND trust_scope=?2 AND state='ready'
                     ORDER BY envelope_id LIMIT 1",
                    params![desired_key, envelope.trust_scope],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )
                .optional()?;
            let active_quarantine = self
                .conn
                .query_row(
                    "SELECT quarantine_id FROM artifact_quarantines
                     WHERE desired_key=?1 AND trust_scope=?2 AND state='active'
                     ORDER BY quarantine_id LIMIT 1",
                    params![desired_key, envelope.trust_scope],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;
            let divergent_incumbent = incumbent
                .as_ref()
                .filter(|(_, tree_root_id)| tree_root_id != &envelope.tree_root_id.0);
            let quarantined = active_quarantine.is_some() || divergent_incumbent.is_some();
            self.conn.execute(
                "INSERT INTO artifact_envelopes(
                    envelope_id, desired_key, trust_scope, tree_root_id, object_id, state,
                    verification_state, created_at, updated_at
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, 'verified', ?7, ?7)
                 ON CONFLICT(envelope_id) DO UPDATE SET updated_at=excluded.updated_at",
                params![
                    envelope_id.0,
                    desired_key,
                    envelope.trust_scope,
                    envelope.tree_root_id.0,
                    object_id.0,
                    if quarantined { "quarantined" } else { "ready" },
                    now_ts(),
                ],
            )?;
            if let Some((incumbent_id, incumbent_tree_id)) = divergent_incumbent {
                let incumbent_envelope_id = ArtifactEnvelopeId::parse(incumbent_id.clone())
                    .map_err(|error| {
                        Error::Corrupt(format!("invalid incumbent envelope ID: {error}"))
                    })?;
                let incumbent_tree_root_id = ArtifactTreeId::parse(incumbent_tree_id.clone())
                    .map_err(|error| {
                        Error::Corrupt(format!("invalid incumbent artifact tree ID: {error}"))
                    })?;
                let evidence = ArtifactDivergenceEvidenceV1 {
                    version: ARTIFACT_DIVERGENCE_EVIDENCE_VERSION,
                    trust_scope: envelope.trust_scope.clone(),
                    desired_key: desired_key.clone(),
                    incumbent_envelope_id: incumbent_envelope_id.clone(),
                    incumbent_tree_root_id,
                    candidate_envelope_id: envelope_id.clone(),
                    candidate_tree_root_id: envelope.tree_root_id.clone(),
                    reason_code: "tree_root_divergence".into(),
                };
                let evidence_object = self.put_object(
                    ARTIFACT_DIVERGENCE_EVIDENCE_KIND,
                    ARTIFACT_DIVERGENCE_EVIDENCE_VERSION,
                    &evidence,
                )?;
                let quarantine_id = crate::ids::ArtifactQuarantineId::new(&cbor(&evidence)?);
                self.conn.execute(
                    "UPDATE artifact_envelopes SET state='quarantined',updated_at=?1
                     WHERE envelope_id IN (?2,?3)",
                    params![now_ts(), incumbent_id, envelope_id.0],
                )?;
                self.conn.execute(
                    "INSERT OR IGNORE INTO artifact_quarantines(
                        quarantine_id,trust_scope,desired_key,incumbent_envelope_id,
                        candidate_envelope_id,reason_code,evidence_object_id,state,created_at
                     ) VALUES(?1,?2,?3,?4,?5,'tree_root_divergence',?6,'active',?7)",
                    params![
                        quarantine_id.0,
                        envelope.trust_scope,
                        desired_key,
                        incumbent_id,
                        envelope_id.0,
                        evidence_object.0,
                        now_ts(),
                    ],
                )?;
                for held_envelope in [incumbent_id, &envelope_id.0] {
                    let hold_id = format!(
                        "hold_{}",
                        crate::ids::short_hash(
                            format!("{}:{held_envelope}", quarantine_id.0).as_bytes(),
                            32,
                        )
                    );
                    self.conn.execute(
                        "INSERT OR IGNORE INTO artifact_holds(
                            hold_id,target_kind,target_id,reason,created_at
                         ) VALUES(?1,'artifact_envelope',?2,?3,?4)",
                        params![hold_id, held_envelope, quarantine_id.0, now_ts()],
                    )?;
                }
            }
            self.create_artifact_attestation_under_write_lock(&envelope_id, &envelope)?;
            Ok(quarantined)
        })();
        match publication {
            Ok(quarantined) => {
                self.conn
                    .execute_batch("RELEASE SAVEPOINT trail_artifact_envelope")?;
                Ok((envelope_id, quarantined))
            }
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT trail_artifact_envelope;
                     RELEASE SAVEPOINT trail_artifact_envelope",
                );
                Err(error)
            }
        }
    }

    pub(crate) fn verify_ready_artifact_envelope_under_write_lock(
        &self,
        envelope_id: &ArtifactEnvelopeId,
        expected_tree_id: &ArtifactTreeId,
    ) -> Result<ArtifactEnvelopeV1> {
        let (desired_key, trust_scope, tree_root_id, state, verification_state) =
            self.conn.query_row(
                "SELECT desired_key, trust_scope, tree_root_id, state, verification_state
             FROM artifact_envelopes WHERE envelope_id=?1",
                params![envelope_id.0],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )?;
        if state != "ready"
            || verification_state != "verified"
            || tree_root_id != expected_tree_id.0
        {
            return Err(Error::Corrupt(format!(
                "artifact envelope `{envelope_id}` is not ready for tree `{expected_tree_id}`"
            )));
        }
        let envelope: ArtifactEnvelopeV1 = self.get_artifact_cas_object(
            &envelope_id.0,
            ARTIFACT_ENVELOPE_KIND,
            ARTIFACT_ENVELOPE_VERSION,
        )?;
        let (actual_id, _) = encode_artifact_envelope(envelope.clone())?;
        if actual_id != *envelope_id || envelope.tree_root_id != *expected_tree_id {
            return Err(Error::Corrupt(format!(
                "artifact envelope `{envelope_id}` has conflicting content identity"
            )));
        }
        let encoded_desired_key = match &envelope.desired_identity {
            ArtifactDesiredIdentityV1::WorkspaceLayerV1 { cache_key, .. } => cache_key,
            ArtifactDesiredIdentityV1::ArtifactDesiredV2 { desired_key } => &desired_key.0,
        };
        if encoded_desired_key != &desired_key || envelope.trust_scope != trust_scope {
            return Err(Error::Corrupt(format!(
                "artifact envelope `{envelope_id}` database identity disagrees with its object"
            )));
        }
        self.validate_envelope_validation_receipts(&envelope)
            .map_err(|error| {
                Error::Corrupt(format!(
                    "artifact envelope `{envelope_id}` has invalid validation evidence: {error}"
                ))
            })?;
        self.verify_artifact_attestations_for_attachment(envelope_id, &envelope)?;
        Ok(envelope)
    }

    pub fn artifact_attestation(
        &self,
        attestation_id: &ArtifactAttestationId,
    ) -> Result<ArtifactAttestationReportV1> {
        let (envelope_id, object_id, producer_identity, trust_scope, state) = self
            .conn
            .query_row(
                "SELECT envelope_id,object_id,producer_identity,trust_scope,state
                 FROM artifact_attestations WHERE attestation_id=?1",
                params![attestation_id.0],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .optional()?
            .ok_or_else(|| Error::ObjectNotFound {
                kind: "artifact attestation",
                id: attestation_id.0.clone(),
            })?;
        let object_id = ObjectId(object_id);
        let attestation: ArtifactAttestationV1 =
            self.get_object(ARTIFACT_ATTESTATION_KIND, &object_id)?;
        let (actual_id, _) = encode_artifact_attestation(attestation.clone())?;
        if actual_id != *attestation_id
            || attestation.statement.envelope_id.0 != envelope_id
            || attestation.statement.producer_identity != producer_identity
            || attestation.statement.trust_scope != trust_scope
        {
            return Err(Error::Corrupt(format!(
                "artifact attestation `{attestation_id}` database identity disagrees with its object"
            )));
        }
        Ok(ArtifactAttestationReportV1 {
            attestation_id: attestation_id.clone(),
            object_id,
            state,
            attestation,
        })
    }

    pub fn artifact_attestations_for_envelope(
        &self,
        envelope_id: &ArtifactEnvelopeId,
    ) -> Result<Vec<ArtifactAttestationReportV1>> {
        let mut statement = self.conn.prepare(
            "SELECT attestation_id FROM artifact_attestations
             WHERE envelope_id=?1 ORDER BY attestation_id",
        )?;
        let ids = statement
            .query_map(params![envelope_id.0], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if ids.len() > MAX_PUBLIC_ARTIFACT_REPORT_ITEMS {
            return Err(Error::InvalidInput(format!(
                "artifact `{envelope_id}` has {} attestations; maximum is {MAX_PUBLIC_ARTIFACT_REPORT_ITEMS}",
                ids.len()
            )));
        }
        ids.into_iter()
            .map(|id| {
                ArtifactAttestationId::parse(id)
                    .map_err(Error::Corrupt)
                    .and_then(|id| self.artifact_attestation(&id))
            })
            .collect()
    }

    pub fn verify_artifact_attestation(
        &self,
        attestation_id: &ArtifactAttestationId,
    ) -> Result<ArtifactAttestationVerificationReportV1> {
        let report = self.artifact_attestation(attestation_id)?;
        let statement = &report.attestation.statement;
        let mut diagnostics = Vec::new();
        let content_identity_valid = encode_artifact_attestation(report.attestation.clone())
            .is_ok_and(|(actual, _)| actual == *attestation_id);
        if !content_identity_valid {
            diagnostics.push("attestation content identity mismatch".into());
        }
        let envelope_binding_valid = self
            .get_artifact_cas_object::<ArtifactEnvelopeV1>(
                &statement.envelope_id.0,
                ARTIFACT_ENVELOPE_KIND,
                ARTIFACT_ENVELOPE_VERSION,
            )
            .is_ok_and(|envelope| artifact_attestation_matches_envelope(statement, &envelope));
        if !envelope_binding_valid {
            diagnostics.push("attestation does not match its exact artifact envelope".into());
        }
        let producer_trusted =
            self.artifact_attestation_producer_trusted(statement, &mut diagnostics);
        let (signature_status, signature_valid) =
            self.verify_artifact_attestation_signature(&report.attestation, &mut diagnostics)?;
        if report.state != "valid" {
            diagnostics.push(format!("attestation database state is `{}`", report.state));
        }
        let valid = report.state == "valid"
            && content_identity_valid
            && envelope_binding_valid
            && producer_trusted
            && signature_valid;
        Ok(ArtifactAttestationVerificationReportV1 {
            attestation_id: attestation_id.clone(),
            envelope_id: statement.envelope_id.clone(),
            state: report.state,
            content_identity_valid,
            envelope_binding_valid,
            producer_trusted,
            signature_status,
            valid,
            diagnostics,
        })
    }

    fn create_artifact_attestation_under_write_lock(
        &self,
        envelope_id: &ArtifactEnvelopeId,
        envelope: &ArtifactEnvelopeV1,
    ) -> Result<ArtifactAttestationId> {
        let statement = self.artifact_attestation_statement(envelope_id, envelope)?;
        let attestation = ArtifactAttestationV1 {
            statement,
            signature: None,
        };
        let (attestation_id, _) = encode_artifact_attestation(attestation.clone())?;
        let object_id = self.put_object(
            ARTIFACT_ATTESTATION_KIND,
            ARTIFACT_ATTESTATION_VERSION,
            &attestation,
        )?;
        self.conn.execute(
            "INSERT INTO artifact_attestations(
                 attestation_id,envelope_id,object_id,producer_identity,trust_scope,state,
                 created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,'valid',?6,?6)
             ON CONFLICT(attestation_id) DO UPDATE SET updated_at=excluded.updated_at",
            params![
                attestation_id.0,
                envelope_id.0,
                object_id.0,
                attestation.statement.producer_identity,
                attestation.statement.trust_scope,
                now_ts(),
            ],
        )?;
        Ok(attestation_id)
    }

    fn artifact_attestation_statement(
        &self,
        envelope_id: &ArtifactEnvelopeId,
        envelope: &ArtifactEnvelopeV1,
    ) -> Result<ArtifactAttestationStatementV1> {
        let mut source_root = None;
        let mut upstream_identities = BTreeMap::new();
        let mut executable_identities = BTreeMap::new();
        let (
            producer_identity,
            producer_trust,
            implementation,
            distribution,
            protocol,
            platform,
            architecture,
        ) = match &envelope.desired_identity {
            ArtifactDesiredIdentityV1::WorkspaceLayerV1 { canonical_key, .. } => {
                source_root = canonical_key
                    .inputs
                    .get("source_root")
                    .filter(|value| value.starts_with("object_"))
                    .map(|value| ObjectId(value.clone()));
                upstream_identities = canonical_key.inputs.clone();
                executable_identities = canonical_key.tool_versions.clone();
                let plugin = self
                    .installed_environment_plugins()?
                    .into_iter()
                    .find(|plugin| {
                        plugin.manifest.adapter.canonical_identity == canonical_key.adapter
                    });
                if let Some(plugin) = plugin {
                    (
                        canonical_key.adapter.clone(),
                        ArtifactProducerTrustTierV1::LocallyTrustedPlugin,
                        plugin.manifest.adapter.implementation_version,
                        plugin.distribution_digest,
                        "trail.environment-adapter/v2".to_string(),
                        canonical_key.platform.clone(),
                        canonical_key.architecture.clone(),
                    )
                } else {
                    (
                        canonical_key.adapter.clone(),
                        ArtifactProducerTrustTierV1::ReviewedBuiltin,
                        canonical_key.adapter_version.to_string(),
                        format!("builtin:{}", canonical_key.adapter),
                        "workspace-layer/v1".to_string(),
                        canonical_key.platform.clone(),
                        canonical_key.architecture.clone(),
                    )
                }
            }
            ArtifactDesiredIdentityV1::ArtifactDesiredV2 { .. } => (
                envelope.component_id.clone(),
                ArtifactProducerTrustTierV1::RepositoryDeclaration,
                "unspecified".to_string(),
                "repository-declaration".to_string(),
                "artifact-envelope/v1".to_string(),
                std::env::consts::OS.to_string(),
                std::env::consts::ARCH.to_string(),
            ),
        };
        let plugin = self
            .installed_environment_plugins()?
            .into_iter()
            .find(|plugin| plugin.manifest.adapter.canonical_identity == producer_identity);
        let (publisher, publisher_key_id) = plugin
            .map(|plugin| (plugin.publisher, plugin.publisher_key_id))
            .unwrap_or((None, None));
        let statement = ArtifactAttestationStatementV1 {
            version: ARTIFACT_ATTESTATION_VERSION,
            envelope_id: envelope_id.clone(),
            desired_identity: envelope.desired_identity.clone(),
            tree_root_id: envelope.tree_root_id.clone(),
            source_root,
            resolution_snapshot_id: envelope.resolution_snapshot_id.clone(),
            upstream_identities,
            producer_identity,
            producer_trust,
            adapter_implementation_version: implementation,
            adapter_distribution_digest: distribution,
            adapter_protocol: protocol,
            publisher,
            publisher_key_id,
            executable_identities,
            platform,
            architecture,
            abi: "host-default".into(),
            capability_ceiling: ArtifactCapabilityCeilingV1::for_phase(
                producer_trust,
                ArtifactExecutionPhaseV1::Construct,
            ),
            sandbox_enforcement: "host-sealed-candidate".into(),
            network_policy: "producer-plan-enforced".into(),
            script_policy: ArtifactScriptPolicyV1::AllowDeclared,
            output_name: envelope.output_name.clone(),
            output_policy: envelope.output_policy,
            portability_scope: envelope.portability_scope.clone(),
            trust_scope: envelope.trust_scope.clone(),
            validation_receipt_ids: envelope.validation_receipt_ids.clone(),
            secret_taint: envelope.secret_taint.clone(),
        };
        validate_artifact_attestation_statement(&statement)?;
        Ok(statement)
    }

    fn verify_artifact_attestations_for_attachment(
        &self,
        envelope_id: &ArtifactEnvelopeId,
        envelope: &ArtifactEnvelopeV1,
    ) -> Result<()> {
        let attestations = self.artifact_attestations_for_envelope(envelope_id)?;
        if attestations.is_empty() {
            return Err(Error::Corrupt(format!(
                "artifact envelope `{envelope_id}` has no host attestation"
            )));
        }
        for attestation in attestations {
            if !artifact_attestation_matches_envelope(&attestation.attestation.statement, envelope)
            {
                return Err(Error::Corrupt(format!(
                    "artifact attestation `{}` does not match envelope `{envelope_id}`",
                    attestation.attestation_id
                )));
            }
            let verification = self.verify_artifact_attestation(&attestation.attestation_id)?;
            if !verification.valid {
                return Err(Error::Corrupt(format!(
                    "artifact attestation `{}` cannot authorize attachment: {}",
                    attestation.attestation_id,
                    verification.diagnostics.join("; ")
                )));
            }
        }
        Ok(())
    }

    fn artifact_attestation_producer_trusted(
        &self,
        statement: &ArtifactAttestationStatementV1,
        diagnostics: &mut Vec<String>,
    ) -> bool {
        if !matches!(
            statement.producer_trust,
            ArtifactProducerTrustTierV1::CertifiedSignedPlugin
                | ArtifactProducerTrustTierV1::LocallyTrustedPlugin
        ) {
            return true;
        }
        let plugins = match self.installed_environment_plugins() {
            Ok(plugins) => plugins,
            Err(error) => {
                diagnostics.push(format!(
                    "producer package or publisher trust cannot be verified: {error}"
                ));
                return false;
            }
        };
        let Some(plugin) = plugins.into_iter().find(|plugin| {
            plugin.manifest.adapter.canonical_identity == statement.producer_identity
        }) else {
            diagnostics.push("producer package is removed or revoked".into());
            return false;
        };
        if plugin.distribution_digest != statement.adapter_distribution_digest {
            diagnostics.push("producer package digest no longer matches the attestation".into());
            return false;
        }
        if plugin.publisher != statement.publisher
            || plugin.publisher_key_id != statement.publisher_key_id
        {
            diagnostics.push("producer publisher identity is removed, revoked, or changed".into());
            return false;
        }
        true
    }

    fn verify_artifact_attestation_signature(
        &self,
        attestation: &ArtifactAttestationV1,
        diagnostics: &mut Vec<String>,
    ) -> Result<(String, bool)> {
        let Some(signature) = &attestation.signature else {
            return Ok(("unsigned".into(), true));
        };
        if signature.algorithm != "ed25519" {
            diagnostics.push("unsupported artifact attestation signature algorithm".into());
            return Ok(("unsupported".into(), false));
        }
        let public_key = match decode_artifact_attestation_hex::<32>(
            &signature.public_key_hex,
            "artifact attestation public key",
        ) {
            Ok(public_key) => public_key,
            Err(error) => {
                diagnostics.push(error.to_string());
                return Ok(("invalid".into(), false));
            }
        };
        let expected_key_id = artifact_attestation_signing_key_id(&public_key);
        if signature.key_id != expected_key_id {
            diagnostics.push("artifact attestation key ID does not match its public key".into());
            return Ok(("invalid".into(), false));
        }
        if self.attestation_key_revocation(&expected_key_id)?.is_some() {
            diagnostics.push("artifact attestation signing key is revoked".into());
            return Ok(("revoked".into(), false));
        }
        let verifying_key = match VerifyingKey::from_bytes(&public_key) {
            Ok(key) => key,
            Err(error) => {
                diagnostics.push(format!("invalid artifact attestation public key: {error}"));
                return Ok(("invalid".into(), false));
            }
        };
        let signature_bytes = match decode_artifact_attestation_hex::<64>(
            &signature.signature_hex,
            "artifact attestation signature",
        ) {
            Ok(signature) => signature,
            Err(error) => {
                diagnostics.push(error.to_string());
                return Ok(("invalid".into(), false));
            }
        };
        let signature = Signature::from_bytes(&signature_bytes);
        let statement_bytes = cbor(&attestation.statement)?;
        if verifying_key.verify(&statement_bytes, &signature).is_err() {
            diagnostics.push("artifact attestation signature verification failed".into());
            return Ok(("invalid".into(), false));
        }
        Ok(("verified".into(), true))
    }

    fn validate_envelope_validation_receipts(&self, envelope: &ArtifactEnvelopeV1) -> Result<()> {
        let mut seen_receipts = BTreeSet::new();
        let mut host_structural_seal = false;
        let mut host_policy_seal = false;
        for receipt_id in &envelope.validation_receipt_ids {
            if !seen_receipts.insert(receipt_id.clone()) {
                return Err(Error::InvalidInput(format!(
                    "artifact envelope repeats validation receipt `{receipt_id}`"
                )));
            }
            let receipt = self.artifact_validation_receipt(receipt_id)?;
            if receipt.desired_identity != envelope.desired_identity
                || receipt.tree_root_id != envelope.tree_root_id
                || receipt.outcome != ArtifactValidationOutcomeV1::Passed
            {
                return Err(Error::InvalidInput(format!(
                    "artifact validation receipt `{receipt_id}` does not pass for the envelope desired identity and tree"
                )));
            }
            if receipt.validator_identity == HOST_WORKSPACE_LAYER_SEAL_VALIDATOR {
                match (&receipt.declaration.kind, receipt.declaration.name.as_str()) {
                    (
                        ArtifactValidationKindV1::Structural,
                        HOST_WORKSPACE_LAYER_STRUCTURAL_SEAL,
                    ) if receipt.declaration.required => {
                        host_structural_seal = true;
                    }
                    (ArtifactValidationKindV1::Policy, HOST_WORKSPACE_LAYER_POLICY_SEAL)
                        if receipt.declaration.required =>
                    {
                        host_policy_seal = true;
                    }
                    _ => {}
                }
            }
        }
        if matches!(
            envelope.desired_identity,
            ArtifactDesiredIdentityV1::WorkspaceLayerV1 { .. }
        ) && (!host_structural_seal || !host_policy_seal)
        {
            return Err(Error::InvalidInput(
                "workspace layer artifact is missing required host structural or policy seal evidence"
                    .into(),
            ));
        }
        Ok(())
    }

    pub fn list_artifact_quarantines(&self) -> Result<Vec<ArtifactQuarantineRecordV1>> {
        let mut statement = self.conn.prepare(
            "SELECT quarantine_id,trust_scope,desired_key,incumbent_envelope_id,
                    candidate_envelope_id,reason_code,evidence_object_id,state,resolution,
                    created_at,resolved_at
             FROM artifact_quarantines ORDER BY created_at,quarantine_id",
        )?;
        let rows = statement
            .query_map([], artifact_quarantine_tuple_from_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if rows.len() > MAX_PUBLIC_ARTIFACT_REPORT_ITEMS {
            return Err(Error::InvalidInput(format!(
                "artifact quarantine report contains {} rows; maximum is {MAX_PUBLIC_ARTIFACT_REPORT_ITEMS}",
                rows.len()
            )));
        }
        rows.into_iter().map(artifact_quarantine_record).collect()
    }

    pub fn artifact_quarantine(
        &self,
        quarantine_id: &ArtifactQuarantineId,
    ) -> Result<ArtifactQuarantineRecordV1> {
        let row = self
            .conn
            .query_row(
                "SELECT quarantine_id,trust_scope,desired_key,incumbent_envelope_id,
                        candidate_envelope_id,reason_code,evidence_object_id,state,resolution,
                        created_at,resolved_at
                 FROM artifact_quarantines WHERE quarantine_id=?1",
                params![quarantine_id.0],
                artifact_quarantine_tuple_from_row,
            )
            .optional()?
            .ok_or_else(|| {
                Error::InvalidInput(format!(
                    "artifact quarantine `{quarantine_id}` does not exist"
                ))
            })?;
        artifact_quarantine_record(row)
    }

    pub fn resolve_artifact_quarantine(
        &self,
        quarantine_id: &ArtifactQuarantineId,
        resolution: ArtifactQuarantineResolutionV1,
    ) -> Result<ArtifactQuarantineRecordV1> {
        let _lock = self.acquire_write_lock()?;
        let record = self.artifact_quarantine(quarantine_id)?;
        if record.state != "active" {
            return Err(Error::InvalidInput(format!(
                "artifact quarantine `{quarantine_id}` is already resolved"
            )));
        }
        let competing = self.conn.query_row(
            "SELECT COUNT(*) FROM artifact_quarantines
             WHERE trust_scope=?1 AND desired_key=?2 AND state='active' AND quarantine_id<>?3",
            params![record.trust_scope, record.desired_key, quarantine_id.0],
            |row| row.get::<_, i64>(0),
        )?;
        if competing != 0 {
            return Err(Error::InvalidInput(format!(
                "artifact quarantine `{quarantine_id}` cannot resolve while {competing} related quarantine(s) remain active"
            )));
        }
        self.conn
            .execute_batch("SAVEPOINT trail_quarantine_resolution")?;
        let resolved = (|| -> Result<()> {
            let accepted = match resolution {
                ArtifactQuarantineResolutionV1::RetainPrivate
                | ArtifactQuarantineResolutionV1::RetireAll => None,
                ArtifactQuarantineResolutionV1::AcceptIncumbent => {
                    Some(record.incumbent_envelope_id.as_ref().ok_or_else(|| {
                        Error::InvalidInput(format!(
                            "artifact quarantine `{quarantine_id}` has no incumbent to accept"
                        ))
                    })?)
                }
                ArtifactQuarantineResolutionV1::AcceptCandidate => {
                    Some(&record.candidate_envelope_id)
                }
            };
            if matches!(resolution, ArtifactQuarantineResolutionV1::RetireAll) {
                self.conn.execute(
                    "UPDATE artifact_envelopes SET state='retired',updated_at=?1
                     WHERE envelope_id=?2 OR envelope_id=?3",
                    params![
                        now_ts(),
                        record.incumbent_envelope_id.as_ref().map(|id| &id.0),
                        record.candidate_envelope_id.0,
                    ],
                )?;
            } else if let Some(accepted) = accepted {
                self.conn.execute(
                    "UPDATE artifact_envelopes SET state='retired',updated_at=?1
                     WHERE envelope_id=?2 OR envelope_id=?3",
                    params![
                        now_ts(),
                        record.incumbent_envelope_id.as_ref().map(|id| &id.0),
                        record.candidate_envelope_id.0,
                    ],
                )?;
                self.conn.execute(
                    "UPDATE artifact_envelopes SET state='ready',updated_at=?1
                     WHERE envelope_id=?2 AND verification_state='verified'",
                    params![now_ts(), accepted.0],
                )?;
            }
            self.conn.execute(
                "UPDATE artifact_quarantines SET state='resolved',resolution=?1,resolved_at=?2
                 WHERE quarantine_id=?3 AND state='active'",
                params![resolution.as_str(), now_ts(), quarantine_id.0],
            )?;
            self.conn.execute(
                "DELETE FROM artifact_holds WHERE reason=?1",
                params![quarantine_id.0],
            )?;
            Ok(())
        })();
        match resolved {
            Ok(()) => self
                .conn
                .execute_batch("RELEASE SAVEPOINT trail_quarantine_resolution")?,
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT trail_quarantine_resolution;
                     RELEASE SAVEPOINT trail_quarantine_resolution",
                );
                return Err(error);
            }
        }
        self.artifact_quarantine(quarantine_id)
    }

    pub fn artifact_quarantine_list_report(&self) -> Result<ArtifactQuarantineListReportV1> {
        let quarantines = self.list_artifact_quarantines()?;
        Ok(ArtifactQuarantineListReportV1 {
            active_count: quarantines
                .iter()
                .filter(|record| record.state == "active")
                .count() as u64,
            resolved_count: quarantines
                .iter()
                .filter(|record| record.state == "resolved")
                .count() as u64,
            quarantines,
        })
    }

    pub fn resolve_artifact_quarantine_report(
        &self,
        quarantine_id: &ArtifactQuarantineId,
        resolution: ArtifactQuarantineResolutionV1,
    ) -> Result<ArtifactQuarantineResolutionReportV1> {
        let quarantine = self.resolve_artifact_quarantine(quarantine_id, resolution)?;
        let mut affected_envelopes = quarantine
            .incumbent_envelope_id
            .iter()
            .cloned()
            .chain(std::iter::once(quarantine.candidate_envelope_id.clone()))
            .collect::<Vec<_>>();
        affected_envelopes.sort();
        affected_envelopes.dedup();
        Ok(ArtifactQuarantineResolutionReportV1 {
            quarantine,
            affected_envelopes,
            recovery_commands: Vec::new(),
        })
    }

    pub(crate) fn materialize_artifact_tree_under_write_lock(
        &self,
        tree_id: &ArtifactTreeId,
        destination: &Path,
    ) -> Result<()> {
        if destination.exists() {
            return Err(Error::InvalidPath {
                path: destination.to_string_lossy().into_owned(),
                reason: "artifact materialization destination already exists".into(),
            });
        }
        let tree: ArtifactTreeRootV1 = self.get_artifact_cas_object(
            &tree_id.0,
            ARTIFACT_TREE_ROOT_KIND,
            ARTIFACT_TREE_ROOT_VERSION,
        )?;
        let (actual_id, _) = encode_artifact_tree_root(tree.clone())?;
        if actual_id != *tree_id {
            return Err(Error::Corrupt(
                "artifact tree cannot materialize because its identity is invalid".into(),
            ));
        }
        // Verify every edge and complete-file digest before exposing a path.
        self.artifact_tree_flat_entries(tree_id)?;
        fs::create_dir(destination)?;
        let materialized =
            self.materialize_artifact_directory(&tree.root_directory_id, destination, 0);
        if let Err(error) = materialized {
            super::workspace_layer::make_tree_writable(destination);
            let _ = fs::remove_dir_all(destination);
            return Err(error);
        }
        super::workspace_layer::sync_layer_tree(destination)?;
        Ok(())
    }

    fn materialize_artifact_directory(
        &self,
        directory_id: &ArtifactTreeId,
        destination: &Path,
        depth: usize,
    ) -> Result<()> {
        if depth > MAX_ARTIFACT_TREE_DEPTH {
            return Err(Error::Corrupt(
                "artifact materialization exceeds the directory-depth bound".into(),
            ));
        }
        let directory: ArtifactDirectoryNodeV1 = self.get_artifact_cas_object(
            &directory_id.0,
            ARTIFACT_DIRECTORY_NODE_KIND,
            ARTIFACT_DIRECTORY_NODE_VERSION,
        )?;
        for entry in directory.entries {
            validate_artifact_entry_name(&entry.name)?;
            let path = destination.join(&entry.name);
            match entry.target {
                ArtifactDirectoryEntryTargetV1::Directory { node_id } => {
                    fs::create_dir(&path)?;
                    self.materialize_artifact_directory(&node_id, &path, depth + 1)?;
                    set_artifact_materialized_mode(&path, 0o755)?;
                }
                ArtifactDirectoryEntryTargetV1::File { node_id } => {
                    let file: ArtifactFileNodeV1 = self.get_artifact_cas_object(
                        &node_id.0,
                        ARTIFACT_FILE_NODE_KIND,
                        ARTIFACT_FILE_NODE_VERSION,
                    )?;
                    let mut output = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)?;
                    match &file.content {
                        ArtifactFileContentV1::Blob { blob_id } => {
                            let blob: ArtifactBlobV1 = self.get_artifact_cas_object(
                                &blob_id.0,
                                ARTIFACT_BLOB_KIND,
                                ARTIFACT_BLOB_VERSION,
                            )?;
                            output.write_all(&blob.bytes)?;
                        }
                        ArtifactFileContentV1::Chunks { chunk_list_id } => {
                            let list: ArtifactChunkListV1 = self.get_artifact_cas_object(
                                &chunk_list_id.0,
                                ARTIFACT_CHUNK_LIST_KIND,
                                ARTIFACT_CHUNK_LIST_VERSION,
                            )?;
                            for chunk_ref in list.chunks {
                                let chunk: ArtifactChunkV1 = self.get_artifact_cas_object(
                                    &chunk_ref.chunk_id.0,
                                    ARTIFACT_CHUNK_KIND,
                                    ARTIFACT_CHUNK_VERSION,
                                )?;
                                output.write_all(&chunk.bytes)?;
                            }
                        }
                    }
                    output.sync_all()?;
                    set_artifact_materialized_mode(&path, file.mode)?;
                }
                ArtifactDirectoryEntryTargetV1::Symlink { target } => {
                    validate_artifact_symlink_target(&target)?;
                    #[cfg(unix)]
                    symlink_file(target, &path)?;
                    #[cfg(windows)]
                    std::os::windows::fs::symlink_file(target, &path)?;
                }
            }
        }
        Ok(())
    }

    fn flatten_artifact_directory(
        &self,
        directory_id: &ArtifactTreeId,
        prefix: &str,
        depth: usize,
        visiting: &mut BTreeSet<ArtifactTreeId>,
        output: &mut BTreeMap<String, ArtifactFlatEntry>,
    ) -> Result<()> {
        if depth > MAX_ARTIFACT_TREE_DEPTH || !visiting.insert(directory_id.clone()) {
            return Err(Error::Corrupt(
                "artifact directory graph is too deep or cyclic".into(),
            ));
        }
        let directory: ArtifactDirectoryNodeV1 = self.get_artifact_cas_object(
            &directory_id.0,
            ARTIFACT_DIRECTORY_NODE_KIND,
            ARTIFACT_DIRECTORY_NODE_VERSION,
        )?;
        let (actual_id, canonical) = encode_artifact_directory_node(directory.clone())?;
        if actual_id != *directory_id
            || from_cbor::<ArtifactDirectoryNodeV1>(&canonical)? != directory
        {
            return Err(Error::Corrupt(format!(
                "artifact directory `{directory_id}` has conflicting encoded identity"
            )));
        }
        for entry in directory.entries {
            let path = if prefix.is_empty() {
                entry.name.clone()
            } else {
                format!("{prefix}/{}", entry.name)
            };
            if output.len() as u64 >= MAX_ARTIFACT_TREE_ENTRIES {
                return Err(Error::Corrupt(
                    "artifact directory graph exceeds its entry bound".into(),
                ));
            }
            match entry.target {
                ArtifactDirectoryEntryTargetV1::Directory { node_id } => {
                    if output
                        .insert(
                            path.clone(),
                            ArtifactFlatEntry {
                                kind: "directory",
                                mode: 0o755,
                                size_bytes: 0,
                                content_hash: None,
                                symlink_target: None,
                            },
                        )
                        .is_some()
                    {
                        return Err(Error::Corrupt("duplicate artifact tree path".into()));
                    }
                    self.flatten_artifact_directory(&node_id, &path, depth + 1, visiting, output)?;
                }
                ArtifactDirectoryEntryTargetV1::File { node_id } => {
                    let file: ArtifactFileNodeV1 = self.get_artifact_cas_object(
                        &node_id.0,
                        ARTIFACT_FILE_NODE_KIND,
                        ARTIFACT_FILE_NODE_VERSION,
                    )?;
                    let (actual_id, _) = encode_artifact_file_node(file.clone())?;
                    if actual_id != node_id {
                        return Err(Error::Corrupt(
                            "artifact file node has conflicting encoded identity".into(),
                        ));
                    }
                    self.verify_artifact_file_content(&file)?;
                    if output
                        .insert(
                            path,
                            ArtifactFlatEntry {
                                kind: "file",
                                mode: file.mode,
                                size_bytes: file.size_bytes,
                                content_hash: Some(file.content_sha256),
                                symlink_target: None,
                            },
                        )
                        .is_some()
                    {
                        return Err(Error::Corrupt("duplicate artifact tree path".into()));
                    }
                }
                ArtifactDirectoryEntryTargetV1::Symlink { target } => {
                    validate_artifact_symlink_within_tree(prefix, &target)?;
                    if output
                        .insert(
                            path,
                            ArtifactFlatEntry {
                                kind: "symlink",
                                mode: 0o777,
                                size_bytes: 0,
                                content_hash: None,
                                symlink_target: Some(target),
                            },
                        )
                        .is_some()
                    {
                        return Err(Error::Corrupt("duplicate artifact tree path".into()));
                    }
                }
            }
        }
        visiting.remove(directory_id);
        Ok(())
    }

    fn verify_artifact_file_content(&self, file: &ArtifactFileNodeV1) -> Result<()> {
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        match &file.content {
            ArtifactFileContentV1::Blob { blob_id } => {
                let blob: ArtifactBlobV1 = self.get_artifact_cas_object(
                    &blob_id.0,
                    ARTIFACT_BLOB_KIND,
                    ARTIFACT_BLOB_VERSION,
                )?;
                let (actual, _) = encode_artifact_blob(blob.clone())?;
                if actual != *blob_id {
                    return Err(Error::Corrupt(
                        "artifact blob identity edge is invalid".into(),
                    ));
                }
                size = blob.bytes.len() as u64;
                hasher.update(blob.bytes);
            }
            ArtifactFileContentV1::Chunks { chunk_list_id } => {
                let list: ArtifactChunkListV1 = self.get_artifact_cas_object(
                    &chunk_list_id.0,
                    ARTIFACT_CHUNK_LIST_KIND,
                    ARTIFACT_CHUNK_LIST_VERSION,
                )?;
                let (actual, _) = encode_artifact_chunk_list(list.clone())?;
                if actual != *chunk_list_id {
                    return Err(Error::Corrupt(
                        "artifact chunk-list identity edge is invalid".into(),
                    ));
                }
                for chunk_ref in list.chunks {
                    let chunk: ArtifactChunkV1 = self.get_artifact_cas_object(
                        &chunk_ref.chunk_id.0,
                        ARTIFACT_CHUNK_KIND,
                        ARTIFACT_CHUNK_VERSION,
                    )?;
                    let (actual, _) = encode_artifact_chunk(chunk.clone())?;
                    if actual != chunk_ref.chunk_id
                        || chunk.bytes.len() as u64 != chunk_ref.size_bytes
                    {
                        return Err(Error::Corrupt(
                            "artifact chunk identity edge is invalid".into(),
                        ));
                    }
                    size = size
                        .checked_add(chunk_ref.size_bytes)
                        .ok_or_else(|| Error::Corrupt("artifact file size edge overflow".into()))?;
                    hasher.update(chunk.bytes);
                }
            }
        }
        if size != file.size_bytes || hex::encode(hasher.finalize()) != file.content_sha256 {
            return Err(Error::Corrupt(
                "artifact file complete size or hash edge is invalid".into(),
            ));
        }
        Ok(())
    }

    pub(crate) fn put_artifact_resolution_snapshot(
        &self,
        mut plan: ArtifactResolutionPlanV1,
        snapshot_bytes: Vec<u8>,
        resolved_identities: BTreeMap<String, String>,
        checksums: BTreeMap<String, String>,
        mut contacted_authorities: Vec<String>,
        refresh: bool,
    ) -> Result<(ObjectId, ArtifactResolutionSnapshotV1)> {
        let _lock = self.acquire_write_lock()?;
        normalize_artifact_resolution_plan(&mut plan)?;
        if !plan.credential_handles.is_empty() {
            return Err(Error::InvalidInput(
                "artifact resolution declares credential access; secret-influenced snapshots cannot enter shared CAS"
                    .into(),
            ));
        }
        if snapshot_bytes.len() as u64 > plan.limits.candidate_bytes {
            return Err(Error::InvalidInput(format!(
                "artifact resolution candidate contains {} bytes; maximum is {}",
                snapshot_bytes.len(),
                plan.limits.candidate_bytes
            )));
        }
        contacted_authorities.sort();
        contacted_authorities.dedup();
        if contacted_authorities.len() > MAX_RESOLUTION_AUTHORITIES
            || contacted_authorities
                .iter()
                .any(|authority| !plan.allowed_authorities.contains(authority))
        {
            return Err(Error::InvalidInput(
                "resolver contacted an undeclared or excessive network authority".into(),
            ));
        }
        validate_identity_map(&resolved_identities, "resolved identity")?;
        validate_identity_map(&checksums, "snapshot checksum")?;

        let current = self.artifact_resolution_snapshot_for_proposal(&plan.proposal_key)?;
        if let Some((current_id, current_snapshot)) = current.as_ref()
            && !refresh
        {
            let candidate_sha256 = sha256_hex(&snapshot_bytes);
            if current_snapshot.content_sha256 != candidate_sha256 {
                return Err(Error::InvalidInput(format!(
                    "proposal `{}` already has pinned snapshot {}; use explicit refresh to replace it",
                    plan.proposal_key, current_id
                )));
            }
            return Ok((current_id.clone(), current_snapshot.clone()));
        }

        let content_sha256 = sha256_hex(&snapshot_bytes);
        let content = ArtifactResolutionContentV1 {
            version: ARTIFACT_RESOLUTION_SNAPSHOT_VERSION,
            content_sha256: content_sha256.clone(),
            bytes: snapshot_bytes,
        };
        let predecessor_snapshot_id = current.map(|(snapshot_id, _)| snapshot_id);
        let snapshot = ArtifactResolutionSnapshotV1 {
            version: ARTIFACT_RESOLUTION_SNAPSHOT_VERSION,
            proposal_key: plan.proposal_key.clone(),
            source_root: plan.source_root.clone(),
            component_id: plan.component_id.clone(),
            adapter_identity: plan.adapter_identity.clone(),
            snapshot_format: plan.snapshot_format.clone(),
            content_object_id: self.put_object(
                ARTIFACT_RESOLUTION_CONTENT_KIND,
                ARTIFACT_RESOLUTION_SNAPSHOT_VERSION,
                &content,
            )?,
            content_sha256,
            resolved_identities,
            checksums,
            resolver_executable_identity: plan.executable_identity,
            policy_identity: plan.policy_identity,
            contacted_authorities,
            predecessor_snapshot_id,
            secret_taint: ArtifactSecretTaintV1::Clear,
            verification_state: ArtifactResolutionVerificationStateV1::Verified,
        };
        validate_artifact_resolution_snapshot(&snapshot)?;
        let snapshot_id = self.put_object(
            ARTIFACT_RESOLUTION_SNAPSHOT_KIND,
            ARTIFACT_RESOLUTION_SNAPSHOT_VERSION,
            &snapshot,
        )?;

        self.conn
            .execute_batch("SAVEPOINT trail_artifact_resolution_snapshot")?;
        let publication = (|| -> Result<()> {
            if refresh {
                self.conn.execute(
                    "UPDATE artifact_resolution_snapshots
                     SET state='superseded', superseded_at=?1
                     WHERE proposal_key=?2 AND state='current'",
                    params![now_ts(), plan.proposal_key],
                )?;
            }
            self.conn.execute(
                "INSERT INTO artifact_resolution_snapshots(
                    snapshot_id, proposal_key, source_root, component_id,
                    adapter_identity, content_object_id, predecessor_snapshot_id,
                    verification_state, state, created_at, superseded_at
                 ) VALUES(?1, ?2, ?3, ?4, ?5, ?6, ?7, 'verified', 'current', ?8, NULL)",
                params![
                    snapshot_id.0,
                    snapshot.proposal_key,
                    snapshot.source_root.0,
                    snapshot.component_id,
                    snapshot.adapter_identity,
                    snapshot.content_object_id.0,
                    snapshot
                        .predecessor_snapshot_id
                        .as_ref()
                        .map(|id| id.0.as_str()),
                    now_ts(),
                ],
            )?;
            Ok(())
        })();
        match publication {
            Ok(()) => self
                .conn
                .execute_batch("RELEASE SAVEPOINT trail_artifact_resolution_snapshot")?,
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT trail_artifact_resolution_snapshot;
                     RELEASE SAVEPOINT trail_artifact_resolution_snapshot",
                );
                return Err(error);
            }
        }
        Ok((snapshot_id, snapshot))
    }

    pub(crate) fn put_artifact_validation_receipt(
        &self,
        receipt: ArtifactValidationReceiptV1,
    ) -> Result<ObjectId> {
        let _lock = self.acquire_write_lock()?;
        self.put_artifact_validation_receipt_under_write_lock(receipt)
    }

    fn put_artifact_validation_receipt_under_write_lock(
        &self,
        receipt: ArtifactValidationReceiptV1,
    ) -> Result<ObjectId> {
        validate_artifact_validation_receipt(&receipt)?;
        self.put_object(
            ARTIFACT_VALIDATION_RECEIPT_KIND,
            ARTIFACT_VALIDATION_RECEIPT_VERSION,
            &receipt,
        )
    }

    fn put_host_workspace_layer_seal_receipt_under_write_lock(
        &self,
        declaration: ArtifactValidationV1,
        desired_identity: ArtifactDesiredIdentityV1,
        tree_root_id: ArtifactTreeId,
        evidence: BTreeMap<String, String>,
    ) -> Result<ObjectId> {
        let outcome = ArtifactValidationOutcomeV1::Passed;
        let validated_input_digest = artifact_validation_receipt_input_digest(
            ARTIFACT_VALIDATION_RECEIPT_VERSION,
            &declaration,
            &desired_identity,
            &tree_root_id,
            HOST_WORKSPACE_LAYER_SEAL_VALIDATOR,
            outcome,
            &evidence,
        )?;
        self.put_artifact_validation_receipt_under_write_lock(ArtifactValidationReceiptV1 {
            version: ARTIFACT_VALIDATION_RECEIPT_VERSION,
            declaration,
            desired_identity,
            tree_root_id,
            validator_identity: HOST_WORKSPACE_LAYER_SEAL_VALIDATOR.into(),
            validated_input_digest,
            outcome,
            evidence,
        })
    }

    pub(crate) fn artifact_validation_receipt(
        &self,
        receipt_id: &ObjectId,
    ) -> Result<ArtifactValidationReceiptV1> {
        let receipt = self.get_object(ARTIFACT_VALIDATION_RECEIPT_KIND, receipt_id)?;
        validate_artifact_validation_receipt(&receipt).map_err(|error| {
            Error::Corrupt(format!(
                "artifact validation receipt `{receipt_id}` is invalid: {error}"
            ))
        })?;
        Ok(receipt)
    }

    pub(crate) fn artifact_resolution_snapshot_for_proposal(
        &self,
        proposal_key: &str,
    ) -> Result<Option<(ObjectId, ArtifactResolutionSnapshotV1)>> {
        validate_resolution_text(proposal_key, "proposal key")?;
        let snapshot_id = self
            .conn
            .query_row(
                "SELECT snapshot_id FROM artifact_resolution_snapshots
                 WHERE proposal_key=?1 AND state='current'",
                params![proposal_key],
                |row| row.get::<_, String>(0),
            )
            .optional()?
            .map(ObjectId);
        let Some(snapshot_id) = snapshot_id else {
            return Ok(None);
        };
        let snapshot = self.get_object(ARTIFACT_RESOLUTION_SNAPSHOT_KIND, &snapshot_id)?;
        validate_artifact_resolution_snapshot(&snapshot)?;
        Ok(Some((snapshot_id, snapshot)))
    }

    pub(crate) fn artifact_resolution_snapshot_for_component(
        &self,
        source_root: &ObjectId,
        component_id: &str,
        adapter_identity: &str,
    ) -> Result<Option<(ObjectId, ArtifactResolutionSnapshotV1)>> {
        let mut statement = self.conn.prepare(
            "SELECT snapshot_id FROM artifact_resolution_snapshots
             WHERE source_root=?1 AND component_id=?2 AND adapter_identity=?3
               AND state='current' AND verification_state='verified'
             ORDER BY proposal_key LIMIT 2",
        )?;
        let snapshot_ids = statement
            .query_map(
                params![source_root.0, component_id, adapter_identity],
                |row| row.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if snapshot_ids.len() > 1 {
            return Err(Error::Corrupt(format!(
                "environment component `{component_id}` has multiple current resolution snapshots for one source root and adapter"
            )));
        }
        let Some(snapshot_id) = snapshot_ids.into_iter().next().map(ObjectId) else {
            return Ok(None);
        };
        let snapshot = self.get_object(ARTIFACT_RESOLUTION_SNAPSHOT_KIND, &snapshot_id)?;
        validate_artifact_resolution_snapshot(&snapshot)?;
        if snapshot.source_root != *source_root
            || snapshot.component_id != component_id
            || snapshot.adapter_identity != adapter_identity
        {
            return Err(Error::Corrupt(format!(
                "resolution snapshot `{snapshot_id}` does not match its component lookup"
            )));
        }
        Ok(Some((snapshot_id, snapshot)))
    }

    pub(crate) fn has_current_artifact_resolution_snapshot(
        &self,
        source_root: &ObjectId,
        component_id: &str,
        adapter_identity: &str,
    ) -> Result<bool> {
        Ok(self.conn.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM artifact_resolution_snapshots
                 WHERE source_root=?1 AND component_id=?2 AND adapter_identity=?3
                   AND state='current' AND verification_state='verified'
             )",
            params![source_root.0, component_id, adapter_identity],
            |row| row.get::<_, bool>(0),
        )?)
    }

    pub(crate) fn artifact_resolution_snapshot_content(
        &self,
        snapshot: &ArtifactResolutionSnapshotV1,
    ) -> Result<Vec<u8>> {
        validate_artifact_resolution_snapshot(snapshot)?;
        let content: ArtifactResolutionContentV1 = self.get_object(
            ARTIFACT_RESOLUTION_CONTENT_KIND,
            &snapshot.content_object_id,
        )?;
        if content.version != ARTIFACT_RESOLUTION_SNAPSHOT_VERSION
            || content.content_sha256 != snapshot.content_sha256
            || sha256_hex(&content.bytes) != snapshot.content_sha256
        {
            return Err(Error::Corrupt(format!(
                "artifact resolution snapshot content {} failed identity verification",
                snapshot.content_object_id
            )));
        }
        Ok(content.bytes)
    }

    pub(crate) fn artifact_resolution_snapshot_content_by_id(
        &self,
        snapshot_id: &ObjectId,
    ) -> Result<(ArtifactResolutionSnapshotV1, Vec<u8>)> {
        let snapshot: ArtifactResolutionSnapshotV1 =
            self.get_object(ARTIFACT_RESOLUTION_SNAPSHOT_KIND, snapshot_id)?;
        validate_artifact_resolution_snapshot(&snapshot)?;
        let bytes = self.artifact_resolution_snapshot_content(&snapshot)?;
        Ok((snapshot, bytes))
    }
}

fn normalized_resolution_authority_evidence(
    plan: &ArtifactResolutionPlanV1,
    mut contacted_authorities: Vec<String>,
) -> Result<ArtifactResolutionAuthorityEvidenceV1> {
    normalize_string_set(
        &mut contacted_authorities,
        MAX_RESOLUTION_AUTHORITIES,
        "contacted authority",
    )?;
    if contacted_authorities
        .iter()
        .any(|authority| !plan.allowed_authorities.contains(authority))
    {
        return Err(Error::InvalidInput(
            "resolver contacted an undeclared network authority".into(),
        ));
    }
    Ok(ArtifactResolutionAuthorityEvidenceV1 {
        allowed_authorities: plan.allowed_authorities.clone(),
        contacted_authorities,
        credential_handles: plan.credential_handles.clone(),
        credential_values_redacted: true,
    })
}

fn redact_resolution_bytes(bytes: &[u8], redactions: &[Vec<u8>]) -> Vec<u8> {
    const REDACTED: &[u8] = b"[REDACTED]";
    let mut output = bytes.to_vec();
    for secret in redactions.iter().filter(|secret| !secret.is_empty()) {
        let mut cursor = 0usize;
        while cursor.saturating_add(secret.len()) <= output.len() {
            let Some(offset) = output[cursor..]
                .windows(secret.len())
                .position(|window| window == secret)
            else {
                break;
            };
            let start = cursor + offset;
            output.splice(start..start + secret.len(), REDACTED.iter().copied());
            cursor = start + REDACTED.len();
        }
    }
    output
}

fn artifact_resolution_attempt_status_str(
    status: ArtifactResolutionAttemptStatusV1,
) -> &'static str {
    match status {
        ArtifactResolutionAttemptStatusV1::Running => "running",
        ArtifactResolutionAttemptStatusV1::Succeeded => "succeeded",
        ArtifactResolutionAttemptStatusV1::Failed => "failed",
        ArtifactResolutionAttemptStatusV1::Cancelled => "cancelled",
        ArtifactResolutionAttemptStatusV1::Abandoned => "abandoned",
    }
}

fn parse_artifact_resolution_attempt_status(
    status: &str,
) -> Result<ArtifactResolutionAttemptStatusV1> {
    match status {
        "running" => Ok(ArtifactResolutionAttemptStatusV1::Running),
        "succeeded" => Ok(ArtifactResolutionAttemptStatusV1::Succeeded),
        "failed" => Ok(ArtifactResolutionAttemptStatusV1::Failed),
        "cancelled" => Ok(ArtifactResolutionAttemptStatusV1::Cancelled),
        "abandoned" => Ok(ArtifactResolutionAttemptStatusV1::Abandoned),
        other => Err(Error::Corrupt(format!(
            "invalid artifact resolution attempt status `{other}`"
        ))),
    }
}

pub(crate) fn normalize_artifact_resolution_plan(
    plan: &mut ArtifactResolutionPlanV1,
) -> Result<()> {
    if plan.version != ARTIFACT_RESOLUTION_PLAN_VERSION {
        return Err(Error::InvalidInput(format!(
            "artifact resolution plan version {} is unsupported",
            plan.version
        )));
    }
    validate_artifact_resolution_capability_ceiling(plan)?;
    for (value, field) in [
        (&plan.proposal_key, "proposal key"),
        (&plan.component_id, "component id"),
        (&plan.adapter_identity, "adapter identity"),
        (&plan.policy_identity, "policy identity"),
        (&plan.program, "program"),
        (&plan.resolved_program, "resolved program"),
        (&plan.executable_identity, "executable identity"),
        (&plan.snapshot_format, "snapshot format"),
    ] {
        validate_resolution_text(value, field)?;
    }
    if plan.argv.is_empty() || plan.argv.len() > MAX_RESOLUTION_ARGV {
        return Err(Error::InvalidInput(format!(
            "artifact resolver argv must contain between 1 and {MAX_RESOLUTION_ARGV} entries"
        )));
    }
    for argument in &plan.argv {
        validate_resolution_text(argument, "resolver argv")?;
    }
    validate_resolution_relative_path(&plan.working_directory, "working directory", true)?;
    validate_resolution_relative_path(&plan.candidate_output, "candidate output", false)?;
    if plan.readable_inputs.is_empty() || plan.readable_inputs.len() > MAX_RESOLUTION_INPUTS {
        return Err(Error::InvalidInput(format!(
            "artifact resolution plan must contain between 1 and {MAX_RESOLUTION_INPUTS} readable inputs"
        )));
    }
    for input in &plan.readable_inputs {
        validate_resolution_relative_path(&input.source_path, "readable input", false)?;
        validate_sha256(&input.content_hash, "readable input hash")?;
    }
    plan.readable_inputs.sort();
    if plan
        .readable_inputs
        .windows(2)
        .any(|pair| pair[0].source_path == pair[1].source_path)
    {
        return Err(Error::InvalidInput(
            "artifact resolution plan contains duplicate readable input paths".into(),
        ));
    }
    normalize_string_set(
        &mut plan.allowed_authorities,
        MAX_RESOLUTION_AUTHORITIES,
        "allowed authority",
    )?;
    normalize_string_set(
        &mut plan.credential_handles,
        MAX_RESOLUTION_CREDENTIAL_HANDLES,
        "credential handle",
    )?;
    if plan.environment_roles.len() > MAX_RESOLUTION_ENVIRONMENT_NAMES {
        return Err(Error::InvalidInput(format!(
            "artifact resolution plan has too many environment roles; maximum is {MAX_RESOLUTION_ENVIRONMENT_NAMES}"
        )));
    }
    for name in plan.environment_roles.keys() {
        if name.is_empty()
            || name.len() > 128
            || !name.chars().enumerate().all(|(index, character)| {
                character == '_'
                    || character.is_ascii_alphanumeric()
                        && (index > 0 || !character.is_ascii_digit())
            })
        {
            return Err(Error::InvalidInput(format!(
                "artifact resolution environment name `{name}` is invalid"
            )));
        }
    }
    if plan.limits.timeout_ms == 0
        || plan.limits.stdout_bytes == 0
        || plan.limits.stderr_bytes == 0
        || plan.limits.candidate_bytes == 0
        || plan.limits.candidate_entries == 0
        || plan.limits.child_processes == 0
    {
        return Err(Error::InvalidInput(
            "artifact resolver limits must all be non-zero".into(),
        ));
    }
    if plan.limits.timeout_ms > MAX_RESOLUTION_TIMEOUT_MS
        || plan.limits.stdout_bytes > MAX_RESOLUTION_CAPTURE_BYTES
        || plan.limits.stderr_bytes > MAX_RESOLUTION_CAPTURE_BYTES
        || plan.limits.candidate_bytes > MAX_RESOLUTION_CANDIDATE_BYTES
        || plan.limits.candidate_entries > MAX_RESOLUTION_CANDIDATE_ENTRIES
        || plan.limits.child_processes > MAX_RESOLUTION_CHILD_PROCESSES
    {
        return Err(Error::InvalidInput(format!(
            "artifact resolver limits exceed host ceilings: timeout_ms<={MAX_RESOLUTION_TIMEOUT_MS}, capture_bytes<={MAX_RESOLUTION_CAPTURE_BYTES}, candidate_bytes<={MAX_RESOLUTION_CANDIDATE_BYTES}, candidate_entries<={MAX_RESOLUTION_CANDIDATE_ENTRIES}, child_processes<={MAX_RESOLUTION_CHILD_PROCESSES}"
        )));
    }
    if plan.validations.is_empty() || plan.validations.len() > MAX_RESOLUTION_VALIDATIONS {
        return Err(Error::InvalidInput(format!(
            "artifact resolution plan must contain between 1 and {MAX_RESOLUTION_VALIDATIONS} validations"
        )));
    }
    for validation in &plan.validations {
        validate_artifact_validation_declaration(validation)?;
    }
    plan.validations.sort();
    if plan
        .validations
        .windows(2)
        .any(|pair| pair[0].name == pair[1].name)
    {
        return Err(Error::InvalidInput(
            "artifact resolution plan contains duplicate validation names".into(),
        ));
    }
    Ok(())
}

fn validate_artifact_resolution_capability_ceiling(plan: &ArtifactResolutionPlanV1) -> Result<()> {
    let ceiling = ArtifactCapabilityCeilingV1::for_phase(
        ArtifactProducerTrustTierV1::RepositoryDeclaration,
        ArtifactExecutionPhaseV1::Resolve,
    );
    if ceiling.publication_authority
        || ceiling.processes != ArtifactProcessCapabilityV1::DeclaredExecutable
        || ceiling.filesystem_read != ArtifactFilesystemReadCapabilityV1::DeclaredInputs
        || ceiling.filesystem_write != ArtifactFilesystemWriteCapabilityV1::IsolatedCandidate
        || (!plan.allowed_authorities.is_empty()
            && ceiling.network != ArtifactNetworkCapabilityV1::ExactAuthorities)
        || (!plan.credential_handles.is_empty()
            && ceiling.secrets != ArtifactSecretCapabilityV1::OpaqueHandles)
    {
        return Err(Error::InvalidInput(format!(
            "artifact resolver plan `{}` exceeds the repository-declaration resolver capability ceiling",
            plan.proposal_key
        )));
    }
    Ok(())
}

pub(crate) fn artifact_desired_key_v2(
    mut material: ArtifactDesiredKeyMaterialV2,
) -> Result<ArtifactDesiredKeyV2> {
    if material.version != ARTIFACT_DESIRED_KEY_MATERIAL_VERSION {
        return Err(Error::InvalidInput(format!(
            "artifact desired-key material version {} is unsupported",
            material.version
        )));
    }
    for (value, field) in [
        (&material.component_id, "component id"),
        (&material.adapter_identity, "adapter identity"),
        (
            &material.adapter_implementation_version,
            "adapter implementation version",
        ),
        (
            &material.adapter_distribution_digest,
            "adapter distribution digest",
        ),
        (&material.adapter_protocol, "adapter protocol"),
        (
            &material.source_closure.normalizer_version,
            "source normalizer",
        ),
        (&material.target, "target"),
        (&material.platform, "platform"),
        (&material.architecture, "architecture"),
        (&material.abi, "ABI"),
        (&material.portability_scope, "portability scope"),
        (&material.trust_scope, "trust scope"),
        (&material.network_policy, "network policy"),
        (&material.sandbox_policy, "sandbox policy"),
    ] {
        validate_resolution_text(value, field)?;
    }
    if !material.source_closure.certified_complete
        && material.source_closure.complete_source_root.is_none()
    {
        return Err(Error::InvalidInput(
            "uncertified artifact source closure must pin the complete source root".into(),
        ));
    }
    if let Some(snapshot_id) = &material.resolution_snapshot_id {
        validate_resolution_text(&snapshot_id.0, "resolution snapshot ID")?;
    }
    if let Some(source_root) = &material.source_closure.complete_source_root {
        validate_resolution_text(&source_root.0, "complete source root")?;
    }
    if material.source_closure.certified_complete
        && material.source_closure.declared_inputs.is_empty()
    {
        return Err(Error::InvalidInput(
            "certified artifact source closure must declare at least one input".into(),
        ));
    }
    if material.source_closure.declared_inputs.len() > MAX_RESOLUTION_INPUTS {
        return Err(Error::InvalidInput(format!(
            "artifact source closure exceeds {MAX_RESOLUTION_INPUTS} declared inputs"
        )));
    }
    for input in &material.source_closure.declared_inputs {
        validate_resolution_relative_path(&input.source_path, "source closure input", false)?;
        validate_sha256(&input.content_hash, "source closure input hash")?;
    }
    material.source_closure.declared_inputs.sort();
    if material
        .source_closure
        .declared_inputs
        .windows(2)
        .any(|pair| pair[0].source_path == pair[1].source_path)
    {
        return Err(Error::InvalidInput(
            "artifact source closure contains duplicate paths".into(),
        ));
    }
    validate_identity_map(&material.upstream_identities, "upstream identity")?;
    if material.actions.is_empty() || material.actions.len() > MAX_RESOLUTION_VALIDATIONS {
        return Err(Error::InvalidInput(
            "artifact desired-key material has an empty or excessive action list".into(),
        ));
    }
    for action in &mut material.actions {
        validate_resolution_text(&action.name, "action name")?;
        validate_resolution_text(&action.executable_identity, "action executable identity")?;
        validate_resolution_relative_path(
            &action.working_directory,
            "action working directory",
            true,
        )?;
        if action.argv.is_empty() || action.argv.len() > MAX_RESOLUTION_ARGV {
            return Err(Error::InvalidInput(
                "artifact action argv is empty or excessive".into(),
            ));
        }
        for argument in &action.argv {
            validate_resolution_text(argument, "action argv")?;
        }
        normalize_string_set(
            &mut action.environment_names,
            MAX_RESOLUTION_ENVIRONMENT_NAMES,
            "action environment name",
        )?;
    }
    material.actions.sort();
    if material
        .actions
        .windows(2)
        .any(|pair| pair[0].name == pair[1].name)
    {
        return Err(Error::InvalidInput(
            "artifact desired-key material contains duplicate action names".into(),
        ));
    }
    if material.outputs.is_empty() || material.outputs.len() > MAX_RESOLUTION_VALIDATIONS {
        return Err(Error::InvalidInput(
            "artifact desired-key material has an empty or excessive output list".into(),
        ));
    }
    for output in &material.outputs {
        validate_resolution_text(&output.name, "output name")?;
        validate_resolution_relative_path(&output.output_path, "output path", false)?;
        validate_resolution_relative_path(&output.mount_path, "output mount path", false)?;
        if let Some(gate) = &output.gate {
            validate_resolution_text(gate, "output gate")?;
        }
        if !material.portability_certified
            && (output.reuse != EnvironmentReuseMode::None
                || output.scope != EnvironmentSharingScope::Lane)
        {
            return Err(Error::InvalidInput(format!(
                "artifact output `{}` lacks portability evidence and must use lane-private, non-reusable policy",
                output.name
            )));
        }
    }
    material.outputs.sort();
    for validation in &material.validations {
        validate_artifact_validation_declaration(validation)?;
    }
    material.validations.sort();
    for export in &material.source_exports {
        validate_resolution_text(&export.name, "source export name")?;
        validate_resolution_text(&export.output_name, "source export output name")?;
        validate_resolution_relative_path(
            &export.artifact_subpath,
            "source export subpath",
            false,
        )?;
        validate_resolution_relative_path(&export.destination, "source export destination", false)?;
        validate_resolution_text(&export.collision_policy, "source export collision policy")?;
        validate_resolution_text(&export.required_validation, "source export validation")?;
        if let Some(gate) = &export.required_gate {
            validate_resolution_text(gate, "source export gate")?;
        }
        validate_resolution_text(&export.authorization_mode, "source export authorization")?;
        if !matches!(export.collision_policy.as_str(), "fail" | "replace")
            || export.authorization_mode != "explicit"
        {
            return Err(Error::InvalidInput(format!(
                "source export `{}` has an unsupported collision or authorization mode",
                export.name
            )));
        }
    }
    material.source_exports.sort();
    if material.build_environment.len() > MAX_RESOLUTION_ENVIRONMENT_NAMES {
        return Err(Error::InvalidInput(
            "artifact build environment contains too many entries".into(),
        ));
    }
    for (name, value) in &material.build_environment {
        validate_resolution_text(name, "build environment name")?;
        validate_resolution_text(value, "build environment value")?;
        if is_sensitive_json_key(name) || contains_sensitive_text(value) {
            return Err(Error::InvalidInput(format!(
                "artifact build environment `{name}` may contain secret material"
            )));
        }
    }
    let canonical = cbor(&material)?;
    Ok(ArtifactDesiredKeyV2::new(&canonical))
}

pub(crate) fn diff_artifact_desired_key_v2(
    previous: &ArtifactDesiredKeyMaterialV2,
    current: &ArtifactDesiredKeyMaterialV2,
) -> Result<ArtifactDesiredKeyDiffV2> {
    let previous_key = artifact_desired_key_v2(previous.clone())?;
    let current_key = artifact_desired_key_v2(current.clone())?;
    let mut edges = Vec::new();

    diff_artifact_scalar(
        "resolution",
        "format_version",
        &previous.version,
        &current.version,
        &mut edges,
    );
    diff_artifact_scalar(
        "resolution",
        "component_id",
        &previous.component_id,
        &current.component_id,
        &mut edges,
    );
    diff_artifact_scalar(
        "resolution",
        "snapshot",
        &previous.resolution_snapshot_id,
        &current.resolution_snapshot_id,
        &mut edges,
    );
    diff_artifact_scalar(
        "resolution",
        "source_closure",
        &previous.source_closure,
        &current.source_closure,
        &mut edges,
    );
    diff_artifact_map(
        "resolution",
        "upstream",
        &previous.upstream_identities,
        &current.upstream_identities,
        &mut edges,
    );

    for (name, left, right) in [
        (
            "adapter_identity",
            &previous.adapter_identity,
            &current.adapter_identity,
        ),
        (
            "adapter_implementation_version",
            &previous.adapter_implementation_version,
            &current.adapter_implementation_version,
        ),
        (
            "adapter_distribution_digest",
            &previous.adapter_distribution_digest,
            &current.adapter_distribution_digest,
        ),
        (
            "adapter_protocol",
            &previous.adapter_protocol,
            &current.adapter_protocol,
        ),
    ] {
        diff_artifact_scalar("tool", name, left, right, &mut edges);
    }
    let previous_actions = previous
        .actions
        .iter()
        .map(|action| (action.name.as_str(), action))
        .collect::<BTreeMap<_, _>>();
    let current_actions = current
        .actions
        .iter()
        .map(|action| (action.name.as_str(), action))
        .collect::<BTreeMap<_, _>>();
    for name in previous_actions
        .keys()
        .chain(current_actions.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let left = previous_actions.get(name);
        let right = current_actions.get(name);
        diff_artifact_scalar(
            "tool",
            &format!("action:{name}:executable"),
            &left.map(|action| &action.executable_identity),
            &right.map(|action| &action.executable_identity),
            &mut edges,
        );
        let left_contract = left.map(|action| {
            (
                action.phase,
                &action.argv,
                &action.working_directory,
                &action.environment_names,
            )
        });
        let right_contract = right.map(|action| {
            (
                action.phase,
                &action.argv,
                &action.working_directory,
                &action.environment_names,
            )
        });
        diff_artifact_scalar("action", name, &left_contract, &right_contract, &mut edges);
    }
    diff_artifact_named_contracts(
        "output",
        &previous.outputs,
        &current.outputs,
        |output| output.name.as_str(),
        &mut edges,
    );
    diff_artifact_named_contracts(
        "validation",
        &previous.validations,
        &current.validations,
        |validation| validation.name.as_str(),
        &mut edges,
    );
    diff_artifact_named_contracts(
        "export",
        &previous.source_exports,
        &current.source_exports,
        |export| export.name.as_str(),
        &mut edges,
    );
    diff_artifact_map(
        "trust",
        "environment",
        &previous.build_environment,
        &current.build_environment,
        &mut edges,
    );
    for (name, left, right) in [
        ("target", &previous.target, &current.target),
        ("platform", &previous.platform, &current.platform),
        (
            "architecture",
            &previous.architecture,
            &current.architecture,
        ),
        ("abi", &previous.abi, &current.abi),
        (
            "portability_scope",
            &previous.portability_scope,
            &current.portability_scope,
        ),
        ("trust_scope", &previous.trust_scope, &current.trust_scope),
        (
            "network_policy",
            &previous.network_policy,
            &current.network_policy,
        ),
    ] {
        diff_artifact_scalar("trust", name, left, right, &mut edges);
    }
    diff_artifact_scalar(
        "trust",
        "portability_certified",
        &previous.portability_certified,
        &current.portability_certified,
        &mut edges,
    );
    diff_artifact_scalar(
        "trust",
        "script_policy",
        &previous.script_policy,
        &current.script_policy,
        &mut edges,
    );
    diff_artifact_scalar(
        "sandbox",
        "sandbox_policy",
        &previous.sandbox_policy,
        &current.sandbox_policy,
        &mut edges,
    );
    edges.sort_by(|left, right| {
        artifact_invalidation_dimension_rank(&left.dimension)
            .cmp(&artifact_invalidation_dimension_rank(&right.dimension))
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left.change.cmp(&right.change))
    });
    edges.dedup();
    Ok(ArtifactDesiredKeyDiffV2 {
        previous_key,
        current_key,
        first: edges.first().cloned(),
        edges,
    })
}

fn artifact_invalidation_dimension_rank(dimension: &str) -> u8 {
    match dimension {
        "resolution" => 0,
        "tool" => 1,
        "action" => 2,
        "output" => 3,
        "validation" => 4,
        "export" => 5,
        "trust" => 6,
        "sandbox" => 7,
        _ => u8::MAX,
    }
}

type ArtifactQuarantineTuple = (
    String,
    String,
    String,
    Option<String>,
    String,
    String,
    String,
    String,
    Option<String>,
    i64,
    Option<i64>,
);

fn artifact_quarantine_tuple_from_row(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<ArtifactQuarantineTuple> {
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
        row.get(9)?,
        row.get(10)?,
    ))
}

fn artifact_quarantine_record(row: ArtifactQuarantineTuple) -> Result<ArtifactQuarantineRecordV1> {
    Ok(ArtifactQuarantineRecordV1 {
        quarantine_id: ArtifactQuarantineId::parse(row.0)
            .map_err(|error| Error::Corrupt(format!("invalid artifact quarantine ID: {error}")))?,
        trust_scope: row.1,
        desired_key: row.2,
        incumbent_envelope_id: row
            .3
            .map(ArtifactEnvelopeId::parse)
            .transpose()
            .map_err(|error| Error::Corrupt(format!("invalid artifact envelope ID: {error}")))?,
        candidate_envelope_id: ArtifactEnvelopeId::parse(row.4)
            .map_err(|error| Error::Corrupt(format!("invalid artifact envelope ID: {error}")))?,
        reason_code: row.5,
        evidence_object_id: ObjectId(row.6),
        state: row.7,
        resolution: row.8,
        created_at: row.9,
        resolved_at: row.10,
    })
}

fn diff_artifact_scalar<T: PartialEq>(
    dimension: &str,
    name: &str,
    previous: &T,
    current: &T,
    edges: &mut Vec<ArtifactInvalidationEdgeV2>,
) {
    if previous != current {
        edges.push(ArtifactInvalidationEdgeV2 {
            dimension: dimension.into(),
            name: name.into(),
            change: "modified".into(),
        });
    }
}

fn diff_artifact_map(
    dimension: &str,
    prefix: &str,
    previous: &BTreeMap<String, String>,
    current: &BTreeMap<String, String>,
    edges: &mut Vec<ArtifactInvalidationEdgeV2>,
) {
    for name in previous
        .keys()
        .chain(current.keys())
        .collect::<BTreeSet<_>>()
    {
        let change = match (previous.get(name), current.get(name)) {
            (None, Some(_)) => Some("added"),
            (Some(_), None) => Some("removed"),
            (Some(left), Some(right)) if left != right => Some("modified"),
            _ => None,
        };
        if let Some(change) = change {
            edges.push(ArtifactInvalidationEdgeV2 {
                dimension: dimension.into(),
                name: format!("{prefix}:{name}"),
                change: change.into(),
            });
        }
    }
}

fn diff_artifact_named_contracts<T: PartialEq>(
    dimension: &str,
    previous: &[T],
    current: &[T],
    name: impl Fn(&T) -> &str,
    edges: &mut Vec<ArtifactInvalidationEdgeV2>,
) {
    let previous = previous
        .iter()
        .map(|item| (name(item), item))
        .collect::<BTreeMap<_, _>>();
    let current = current
        .iter()
        .map(|item| (name(item), item))
        .collect::<BTreeMap<_, _>>();
    for item_name in previous
        .keys()
        .chain(current.keys())
        .copied()
        .collect::<BTreeSet<_>>()
    {
        let change = match (previous.get(item_name), current.get(item_name)) {
            (None, Some(_)) => Some("added"),
            (Some(_), None) => Some("removed"),
            (Some(left), Some(right)) if left != right => Some("modified"),
            _ => None,
        };
        if let Some(change) = change {
            edges.push(ArtifactInvalidationEdgeV2 {
                dimension: dimension.into(),
                name: item_name.into(),
                change: change.into(),
            });
        }
    }
}

fn encode_artifact_directory_node(
    mut node: ArtifactDirectoryNodeV1,
) -> Result<(ArtifactTreeId, Vec<u8>)> {
    if node.version != ARTIFACT_DIRECTORY_NODE_VERSION {
        return Err(Error::InvalidInput(
            "artifact directory node has an unsupported version".into(),
        ));
    }
    node.entries.sort();
    for entry in &node.entries {
        validate_artifact_entry_name(&entry.name)?;
        if let ArtifactDirectoryEntryTargetV1::Symlink { target } = &entry.target {
            validate_artifact_symlink_target(target)?;
        }
    }
    if node
        .entries
        .windows(2)
        .any(|pair| pair[0].name == pair[1].name)
    {
        return Err(Error::InvalidInput(
            "artifact directory contains duplicate entry names".into(),
        ));
    }
    let bytes = cbor(&node)?;
    Ok((
        artifact_tree_id(ARTIFACT_DIRECTORY_NODE_KIND, &bytes),
        bytes,
    ))
}

fn canonical_artifact_directory_node(
    node: ArtifactDirectoryNodeV1,
) -> Result<ArtifactDirectoryNodeV1> {
    let (_, bytes) = encode_artifact_directory_node(node)?;
    from_cbor(&bytes)
}

fn decode_artifact_directory_node(bytes: &[u8]) -> Result<ArtifactDirectoryNodeV1> {
    let node: ArtifactDirectoryNodeV1 = from_cbor(bytes)?;
    let (_, canonical) = encode_artifact_directory_node(node.clone())?;
    if canonical != bytes {
        return Err(Error::Corrupt(
            "artifact directory node is not canonically ordered".into(),
        ));
    }
    Ok(node)
}

fn encode_artifact_blob(blob: ArtifactBlobV1) -> Result<(ArtifactBlobId, Vec<u8>)> {
    if blob.version != ARTIFACT_BLOB_VERSION || sha256_hex(&blob.bytes) != blob.content_sha256 {
        return Err(Error::InvalidInput(
            "artifact blob version or complete content hash is invalid".into(),
        ));
    }
    let bytes = cbor(&blob)?;
    Ok((ArtifactBlobId::new(&blob.bytes), bytes))
}

fn decode_artifact_blob(bytes: &[u8], expected: &ArtifactBlobId) -> Result<ArtifactBlobV1> {
    let blob: ArtifactBlobV1 = from_cbor(bytes)?;
    let (actual, canonical) = encode_artifact_blob(blob.clone())?;
    if &actual != expected || canonical != bytes {
        return Err(Error::Corrupt(
            "artifact blob content identity or canonical encoding is invalid".into(),
        ));
    }
    Ok(blob)
}

fn encode_artifact_chunk(chunk: ArtifactChunkV1) -> Result<(ArtifactChunkId, Vec<u8>)> {
    if chunk.version != ARTIFACT_CHUNK_VERSION || sha256_hex(&chunk.bytes) != chunk.content_sha256 {
        return Err(Error::InvalidInput(
            "artifact chunk version or content hash is invalid".into(),
        ));
    }
    let bytes = cbor(&chunk)?;
    Ok((ArtifactChunkId::new(&chunk.bytes), bytes))
}

fn encode_artifact_chunk_list(list: ArtifactChunkListV1) -> Result<(ArtifactChunkListId, Vec<u8>)> {
    if list.version != ARTIFACT_CHUNK_LIST_VERSION
        || list.algorithm != "fastcdc-v1"
        || list.chunks.is_empty()
        || list.chunks.iter().any(|chunk| chunk.size_bytes == 0)
        || list
            .chunks
            .iter()
            .try_fold(0u64, |total, chunk| total.checked_add(chunk.size_bytes))
            != Some(list.file_size_bytes)
    {
        return Err(Error::InvalidInput(
            "artifact chunk list has invalid version, algorithm, or size edges".into(),
        ));
    }
    validate_sha256(&list.file_sha256, "chunk-list file hash")?;
    let bytes = cbor(&list)?;
    Ok((
        ArtifactChunkListId::new(&artifact_identity_seed(ARTIFACT_CHUNK_LIST_KIND, &bytes)),
        bytes,
    ))
}

fn encode_artifact_file_node(node: ArtifactFileNodeV1) -> Result<(ArtifactFileId, Vec<u8>)> {
    if node.version != ARTIFACT_FILE_NODE_VERSION
        || node.mode & !0o777 != 0
        || node.executable != (node.mode & 0o111 != 0)
    {
        return Err(Error::InvalidInput(
            "artifact file node has invalid version or normalized mode".into(),
        ));
    }
    validate_sha256(&node.content_sha256, "file content hash")?;
    let bytes = cbor(&node)?;
    Ok((
        ArtifactFileId::new(&artifact_identity_seed(ARTIFACT_FILE_NODE_KIND, &bytes)),
        bytes,
    ))
}

fn encode_artifact_tree_root(root: ArtifactTreeRootV1) -> Result<(ArtifactTreeId, Vec<u8>)> {
    if root.version != ARTIFACT_TREE_ROOT_VERSION || root.path_normalizer != "trail-paths/v1" {
        return Err(Error::InvalidInput(
            "artifact tree root has invalid version or path normalizer".into(),
        ));
    }
    let bytes = cbor(&root)?;
    Ok((artifact_tree_id(ARTIFACT_TREE_ROOT_KIND, &bytes), bytes))
}

fn encode_artifact_envelope(
    mut envelope: ArtifactEnvelopeV1,
) -> Result<(ArtifactEnvelopeId, Vec<u8>)> {
    if envelope.version != ARTIFACT_ENVELOPE_VERSION {
        return Err(Error::InvalidInput(
            "artifact envelope has an unsupported version".into(),
        ));
    }
    validate_resolution_text(&envelope.component_id, "envelope component id")?;
    validate_resolution_text(&envelope.output_name, "envelope output name")?;
    validate_resolution_text(&envelope.portability_scope, "envelope portability scope")?;
    validate_resolution_text(&envelope.trust_scope, "envelope trust scope")?;
    validate_artifact_secret_taint(&envelope.secret_taint)?;
    envelope.validation_receipt_ids.sort();
    envelope.validation_receipt_ids.dedup();
    let bytes = cbor(&envelope)?;
    Ok((
        ArtifactEnvelopeId::new(&artifact_identity_seed(ARTIFACT_ENVELOPE_KIND, &bytes)),
        bytes,
    ))
}

fn encode_artifact_attestation(
    attestation: ArtifactAttestationV1,
) -> Result<(ArtifactAttestationId, Vec<u8>)> {
    validate_artifact_attestation_statement(&attestation.statement)?;
    if let Some(signature) = &attestation.signature {
        validate_resolution_text(&signature.algorithm, "attestation signature algorithm")?;
        validate_resolution_text(&signature.key_id, "attestation signature key id")?;
        decode_artifact_attestation_hex::<32>(
            &signature.public_key_hex,
            "artifact attestation public key",
        )?;
        decode_artifact_attestation_hex::<64>(
            &signature.signature_hex,
            "artifact attestation signature",
        )?;
    }
    let bytes = cbor(&attestation)?;
    Ok((ArtifactAttestationId::new(&bytes), bytes))
}

fn validate_artifact_attestation_statement(
    statement: &ArtifactAttestationStatementV1,
) -> Result<()> {
    if statement.version != ARTIFACT_ATTESTATION_VERSION {
        return Err(Error::InvalidInput(
            "artifact attestation has an unsupported version".into(),
        ));
    }
    for (value, field) in [
        (
            &statement.producer_identity,
            "attestation producer identity",
        ),
        (
            &statement.adapter_implementation_version,
            "attestation adapter implementation version",
        ),
        (
            &statement.adapter_distribution_digest,
            "attestation adapter distribution digest",
        ),
        (&statement.adapter_protocol, "attestation adapter protocol"),
        (&statement.platform, "attestation platform"),
        (&statement.architecture, "attestation architecture"),
        (&statement.abi, "attestation ABI"),
        (
            &statement.sandbox_enforcement,
            "attestation sandbox enforcement",
        ),
        (&statement.network_policy, "attestation network policy"),
        (&statement.output_name, "attestation output name"),
        (
            &statement.portability_scope,
            "attestation portability scope",
        ),
        (&statement.trust_scope, "attestation trust scope"),
    ] {
        validate_resolution_text(value, field)?;
    }
    for (value, field) in [
        (statement.publisher.as_deref(), "attestation publisher"),
        (
            statement.publisher_key_id.as_deref(),
            "attestation publisher key id",
        ),
    ] {
        if let Some(value) = value {
            validate_resolution_text(value, field)?;
        }
    }
    validate_attestation_identity_map(
        &statement.upstream_identities,
        "attestation upstream identity",
    )?;
    validate_attestation_identity_map(
        &statement.executable_identities,
        "attestation executable identity",
    )?;
    if statement.validation_receipt_ids.len() > MAX_RESOLUTION_VALIDATIONS
        || !statement
            .validation_receipt_ids
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    {
        return Err(Error::InvalidInput(
            "artifact attestation validation receipts are excessive, duplicated, or not canonical"
                .into(),
        ));
    }
    validate_artifact_secret_taint(&statement.secret_taint)?;
    if !statement.secret_taint.is_clear() {
        return Err(Error::InvalidInput(
            "secret-tainted output cannot be attested for shared attachment".into(),
        ));
    }
    if statement.capability_ceiling.publication_authority
        || statement.capability_ceiling.producer_trust != statement.producer_trust
    {
        return Err(Error::InvalidInput(
            "artifact attestation capability evidence is inconsistent or grants publication authority"
                .into(),
        ));
    }
    Ok(())
}

fn validate_attestation_identity_map(values: &BTreeMap<String, String>, field: &str) -> Result<()> {
    if values.len() > MAX_RESOLUTION_INPUTS {
        return Err(Error::InvalidInput(format!(
            "artifact {field} count exceeds {MAX_RESOLUTION_INPUTS}"
        )));
    }
    for (name, value) in values {
        validate_resolution_text(name, field)?;
        if value.len() > MAX_RESOLUTION_TEXT_BYTES
            || value
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\r' | '\t'))
        {
            return Err(Error::InvalidInput(format!(
                "artifact {field} value is oversized or contains control characters"
            )));
        }
        if is_sensitive_json_key(name) || contains_sensitive_text(value) {
            return Err(Error::InvalidInput(format!(
                "artifact {field} may contain secret material"
            )));
        }
    }
    Ok(())
}

fn artifact_attestation_matches_envelope(
    statement: &ArtifactAttestationStatementV1,
    envelope: &ArtifactEnvelopeV1,
) -> bool {
    statement.desired_identity == envelope.desired_identity
        && statement.tree_root_id == envelope.tree_root_id
        && statement.resolution_snapshot_id == envelope.resolution_snapshot_id
        && statement.output_name == envelope.output_name
        && statement.output_policy == envelope.output_policy
        && statement.portability_scope == envelope.portability_scope
        && statement.trust_scope == envelope.trust_scope
        && statement.validation_receipt_ids == envelope.validation_receipt_ids
        && statement.secret_taint == envelope.secret_taint
}

fn decode_artifact_attestation_hex<const N: usize>(value: &str, field: &str) -> Result<[u8; N]> {
    let bytes = hex::decode(value)
        .map_err(|error| Error::InvalidInput(format!("invalid {field}: {error}")))?;
    bytes
        .try_into()
        .map_err(|_| Error::InvalidInput(format!("invalid {field} length")))
}

fn artifact_attestation_signing_key_id(public_key: &[u8; 32]) -> String {
    format!(
        "attestation_key_{}",
        sha256_hex(
            &[
                b"trail-artifact-attestation-key-v1\0".as_slice(),
                public_key
            ]
            .concat()
        )
    )
}

fn artifact_tree_id(kind: &str, bytes: &[u8]) -> ArtifactTreeId {
    ArtifactTreeId::new(&artifact_identity_seed(kind, bytes))
}

fn artifact_identity_seed(kind: &str, bytes: &[u8]) -> Vec<u8> {
    let mut seed = Vec::with_capacity(kind.len() + bytes.len() + 10);
    seed.extend_from_slice(kind.as_bytes());
    seed.push(0);
    seed.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    seed.extend_from_slice(bytes);
    seed
}

fn validate_artifact_entry_name(name: &str) -> Result<()> {
    let normalized = normalize_relative_path(name)?;
    if normalized != name || name.contains('/') {
        return Err(Error::InvalidInput(format!(
            "artifact directory entry `{name}` is not one normalized path component"
        )));
    }
    Ok(())
}

fn validate_artifact_symlink_target(target: &str) -> Result<()> {
    if target.is_empty()
        || target.len() > MAX_RESOLUTION_TEXT_BYTES
        || Path::new(target).is_absolute()
        || target.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput(
            "artifact symlink target is empty, absolute, oversized, or contains controls".into(),
        ));
    }
    Ok(())
}

fn validate_artifact_symlink_within_tree(parent: &str, target: &str) -> Result<()> {
    let link_path = if parent.is_empty() {
        "link".to_string()
    } else {
        format!("{parent}/link")
    };
    resolve_artifact_symlink_path(&link_path, target).map(|_| ())
}

fn resolve_artifact_symlink_path(link_path: &str, target: &str) -> Result<String> {
    validate_artifact_symlink_target(target)?;
    let parent = link_path.rsplit_once('/').map_or("", |(parent, _)| parent);
    let mut components = parent
        .split('/')
        .filter(|component| !component.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for component in Path::new(target).components() {
        match component {
            Component::Normal(component) => {
                let component = component.to_str().ok_or_else(|| Error::InvalidPath {
                    path: target.into(),
                    reason: "artifact symlink target must be valid Unicode".into(),
                })?;
                validate_artifact_entry_name(component)?;
                components.push(component.to_string());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                if components.pop().is_none() {
                    return Err(Error::InvalidPath {
                        path: target.into(),
                        reason: "artifact symlink escapes the tree root".into(),
                    });
                }
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(Error::InvalidPath {
                    path: target.into(),
                    reason: "artifact symlink target must be relative".into(),
                });
            }
        }
    }
    Ok(components.join("/"))
}

fn ensure_artifact_file_unchanged(
    path: &Path,
    before: &fs::Metadata,
    after: &fs::Metadata,
    observed_bytes: u64,
) -> Result<()> {
    if observed_bytes != before.len() || !same_artifact_metadata(before, after) {
        return Err(Error::InvalidInput(format!(
            "artifact file `{}` changed during ingestion",
            path.display()
        )));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ArtifactSecretPolicy {
    Strict,
    LockedPublicDependencies,
}

fn validate_artifact_secret_policy(
    bytes: &[u8],
    relative_path: Option<&str>,
    policy: ArtifactSecretPolicy,
) -> Result<()> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(());
    };
    let contains_private_key = {
        let upper = text.to_ascii_uppercase();
        upper.contains("-----BEGIN ") && upper.contains("PRIVATE KEY-----")
    };
    let sensitive = match relative_path {
        Some(path) if policy == ArtifactSecretPolicy::LockedPublicDependencies => {
            is_secret_bearing_artifact_path(path)
                && (contains_private_key || contains_sensitive_text(text))
        }
        Some(path) => {
            contains_private_key
                || is_secret_bearing_artifact_path(path) && contains_sensitive_text(text)
        }
        None => contains_sensitive_text(text),
    };
    if sensitive {
        let path = relative_path
            .map(|path| format!(" `{path}`"))
            .unwrap_or_default();
        return Err(Error::InvalidInput(format!(
            "artifact content{path} may contain secret material and cannot enter shared CAS"
        )));
    }
    Ok(())
}

fn is_secret_bearing_artifact_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path).to_ascii_lowercase();
    name == ".npmrc"
        || name == ".pypirc"
        || name == ".netrc"
        || name == "credentials"
        || name == "credentials.json"
        || name == "secrets"
        || name == "secrets.json"
        || name == "id_rsa"
        || name == "id_ed25519"
        || name == ".env"
        || name.starts_with(".env.")
        || name.ends_with(".env")
        || name.ends_with(".pem")
        || name.ends_with(".key")
}

fn validate_artifact_metadata_policy(path: &Path, metadata: &fs::Metadata) -> Result<()> {
    #[cfg(unix)]
    {
        if metadata.permissions().mode() & 0o7000 != 0 {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().into_owned(),
                reason: "setuid, setgid, and sticky artifact modes are prohibited".into(),
            });
        }
        if metadata.is_file()
            && metadata.len() > ARTIFACT_WHOLE_BLOB_MAX_BYTES as u64
            && metadata.blocks().saturating_mul(512) < metadata.len() / 2
        {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().into_owned(),
                reason: "excessively sparse artifact files are prohibited".into(),
            });
        }
        let mut attributes = xattr::list(path)?
            .filter(|attribute| attribute != "com.apple.provenance")
            .collect::<Vec<_>>();
        attributes.sort();
        if let Some(attribute) = attributes.first() {
            return Err(Error::InvalidPath {
                path: path.to_string_lossy().into_owned(),
                reason: format!(
                    "artifact extended attribute `{}` is prohibited",
                    attribute.to_string_lossy()
                ),
            });
        }
    }
    Ok(())
}

fn same_artifact_metadata(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len()
        && left.file_type() == right.file_type()
        && left.modified().ok() == right.modified().ok()
}

#[cfg(unix)]
fn normalized_artifact_file_mode(metadata: &fs::Metadata) -> u32 {
    if metadata.permissions().mode() & 0o111 != 0 {
        0o755
    } else {
        0o644
    }
}

#[cfg(not(unix))]
fn normalized_artifact_file_mode(_metadata: &fs::Metadata) -> u32 {
    0o644
}

#[cfg(unix)]
fn set_artifact_materialized_mode(path: &Path, mode: u32) -> Result<()> {
    fs::set_permissions(path, fs::Permissions::from_mode(mode))?;
    Ok(())
}

#[cfg(not(unix))]
fn set_artifact_materialized_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

fn validate_artifact_resolution_snapshot(snapshot: &ArtifactResolutionSnapshotV1) -> Result<()> {
    if snapshot.version != ARTIFACT_RESOLUTION_SNAPSHOT_VERSION {
        return Err(Error::Corrupt(format!(
            "artifact resolution snapshot version {} is unsupported",
            snapshot.version
        )));
    }
    validate_resolution_text(&snapshot.proposal_key, "snapshot proposal key")?;
    validate_resolution_text(&snapshot.component_id, "snapshot component id")?;
    validate_resolution_text(&snapshot.adapter_identity, "snapshot adapter identity")?;
    validate_resolution_text(&snapshot.snapshot_format, "snapshot format")?;
    validate_resolution_text(
        &snapshot.resolver_executable_identity,
        "snapshot resolver executable identity",
    )?;
    validate_resolution_text(&snapshot.policy_identity, "snapshot policy identity")?;
    validate_sha256(&snapshot.content_sha256, "snapshot content hash")?;
    validate_identity_map(&snapshot.resolved_identities, "resolved identity")?;
    validate_identity_map(&snapshot.checksums, "snapshot checksum")?;
    validate_artifact_secret_taint(&snapshot.secret_taint)?;
    if !snapshot.secret_taint.is_clear() {
        return Err(Error::Corrupt(
            "secret-tainted artifact resolution snapshot entered shared storage".into(),
        ));
    }
    if snapshot.contacted_authorities.len() > MAX_RESOLUTION_AUTHORITIES
        || !snapshot
            .contacted_authorities
            .windows(2)
            .all(|pair| pair[0] < pair[1])
    {
        return Err(Error::Corrupt(
            "artifact snapshot authorities are excessive, duplicated, or not canonical".into(),
        ));
    }
    Ok(())
}

fn resolution_is_secret_tainted(plan: &ArtifactResolutionPlanV1, redactions: &[Vec<u8>]) -> bool {
    !plan.credential_handles.is_empty() || redactions.iter().any(|secret| !secret.is_empty())
}

fn artifact_secret_taint(secret_tainted: bool, channel: &str) -> ArtifactSecretTaintV1 {
    if secret_tainted {
        ArtifactSecretTaintV1::Tainted {
            channels: vec![channel.to_string()],
        }
    } else {
        ArtifactSecretTaintV1::Clear
    }
}

pub(super) fn validate_artifact_secret_taint(taint: &ArtifactSecretTaintV1) -> Result<()> {
    let ArtifactSecretTaintV1::Tainted { channels } = taint else {
        return Ok(());
    };
    if channels.is_empty()
        || channels.len() > MAX_RESOLUTION_ENVIRONMENT_NAMES
        || !channels.windows(2).all(|pair| pair[0] < pair[1])
    {
        return Err(Error::InvalidInput(
            "artifact secret-taint channels are empty, excessive, duplicated, or not canonical"
                .into(),
        ));
    }
    for channel in channels {
        validate_resolution_text(channel, "secret-taint channel")?;
        if contains_sensitive_text(channel) {
            return Err(Error::InvalidInput(
                "artifact secret-taint metadata may identify a channel but cannot contain secret material"
                    .into(),
            ));
        }
    }
    Ok(())
}

fn normalize_string_set(values: &mut Vec<String>, maximum: usize, field: &str) -> Result<()> {
    if values.len() > maximum {
        return Err(Error::InvalidInput(format!(
            "artifact resolution {field} count exceeds {maximum}"
        )));
    }
    for value in values.iter() {
        validate_resolution_text(value, field)?;
    }
    values.sort();
    values.dedup();
    Ok(())
}

fn validate_identity_map(values: &BTreeMap<String, String>, field: &str) -> Result<()> {
    if values.len() > MAX_RESOLUTION_INPUTS {
        return Err(Error::InvalidInput(format!(
            "artifact {field} count exceeds {MAX_RESOLUTION_INPUTS}"
        )));
    }
    for (key, value) in values {
        validate_resolution_text(key, field)?;
        validate_resolution_text(value, field)?;
    }
    Ok(())
}

fn validate_artifact_validation_declaration(validation: &ArtifactValidationV1) -> Result<()> {
    validate_resolution_text(&validation.name, "validation name")?;
    validate_identity_map(&validation.parameters, "validation parameter")
}

pub(crate) fn validate_artifact_validation_receipt(
    receipt: &ArtifactValidationReceiptV1,
) -> Result<()> {
    if receipt.version != ARTIFACT_VALIDATION_RECEIPT_VERSION {
        return Err(Error::InvalidInput(format!(
            "artifact validation receipt version {} is unsupported",
            receipt.version
        )));
    }
    validate_artifact_validation_declaration(&receipt.declaration)?;
    validate_resolution_text(&receipt.validator_identity, "validator identity")?;
    validate_sha256(&receipt.validated_input_digest, "validated input digest")?;
    validate_identity_map(&receipt.evidence, "validation evidence")?;
    for (name, value) in &receipt.evidence {
        if is_sensitive_json_key(name) || contains_sensitive_text(value) {
            return Err(Error::InvalidInput(format!(
                "artifact validation evidence `{name}` may contain secret material"
            )));
        }
    }
    if receipt.validator_identity == HOST_WORKSPACE_LAYER_SEAL_VALIDATOR {
        let expected = artifact_validation_receipt_input_digest(
            receipt.version,
            &receipt.declaration,
            &receipt.desired_identity,
            &receipt.tree_root_id,
            &receipt.validator_identity,
            receipt.outcome,
            &receipt.evidence,
        )?;
        if receipt.validated_input_digest != expected {
            return Err(Error::InvalidInput(
                "host workspace layer validation receipt has a stale input digest".into(),
            ));
        }
    }
    Ok(())
}

fn artifact_validation_receipt_input_digest(
    version: u16,
    declaration: &ArtifactValidationV1,
    desired_identity: &ArtifactDesiredIdentityV1,
    tree_root_id: &ArtifactTreeId,
    validator_identity: &str,
    outcome: ArtifactValidationOutcomeV1,
    evidence: &BTreeMap<String, String>,
) -> Result<String> {
    Ok(sha256_hex(&serde_json::to_vec(&(
        version,
        declaration,
        desired_identity,
        tree_root_id,
        validator_identity,
        outcome,
        evidence,
    ))?))
}

fn validate_resolution_relative_path(value: &str, field: &str, allow_dot: bool) -> Result<()> {
    if allow_dot && value == "." {
        return Ok(());
    }
    let normalized = normalize_relative_path(value)?;
    if normalized != value {
        return Err(Error::InvalidInput(format!(
            "artifact resolution {field} `{value}` is not normalized"
        )));
    }
    Ok(())
}

fn validate_resolution_text(value: &str, field: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > MAX_RESOLUTION_TEXT_BYTES
        || value.chars().any(char::is_control)
    {
        return Err(Error::InvalidInput(format!(
            "artifact resolution {field} is empty, oversized, or contains control characters"
        )));
    }
    Ok(())
}

fn artifact_materialization_backend_compatibility() -> String {
    format!(
        "trail-real-directory/v1/{}/{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

fn real_artifact_materialization_directory_exists(path: &Path, label: &str) -> Result<bool> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            Err(Error::InvalidPath {
                path: path.to_string_lossy().into_owned(),
                reason: format!("{label} must be a real directory inside Trail storage"),
            })
        }
        Ok(_) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::Io(error)),
    }
}

fn validate_sha256(value: &str, field: &str) -> Result<()> {
    if value.len() != 64 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(Error::InvalidInput(format!(
            "artifact resolution {field} must be a SHA-256 hex digest"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::time::Instant;

    fn fixture_plan(source_root: ObjectId) -> ArtifactResolutionPlanV1 {
        ArtifactResolutionPlanV1 {
            version: ARTIFACT_RESOLUTION_PLAN_VERSION,
            proposal_key: "proposal_fixture".into(),
            source_root,
            component_id: "cargo:root".into(),
            adapter_identity: "trail.builtin/cargo@1".into(),
            policy_identity: "policy_fixture".into(),
            program: "cargo".into(),
            resolved_program: "/usr/bin/cargo".into(),
            executable_identity: "sha256:fixture".into(),
            argv: vec!["cargo".into(), "generate-lockfile".into()],
            working_directory: ".".into(),
            readable_inputs: vec![ArtifactResolutionInputV1 {
                source_path: "Cargo.toml".into(),
                content_hash: "11".repeat(32),
                size_bytes: 12,
            }],
            candidate_output: "candidate/Cargo.lock".into(),
            allowed_authorities: vec!["index.crates.io:443".into()],
            credential_handles: Vec::new(),
            script_policy: ArtifactScriptPolicyV1::Deny,
            environment_roles: BTreeMap::from([(
                "CARGO_HOME".into(),
                ArtifactEnvironmentRoleV1::Runtime,
            )]),
            limits: ArtifactActionLimitsV1 {
                timeout_ms: 60_000,
                stdout_bytes: 64 * 1024,
                stderr_bytes: 64 * 1024,
                candidate_bytes: 1024 * 1024,
                candidate_entries: 1,
                child_processes: 8,
            },
            snapshot_format: "cargo-lock/v4".into(),
            validations: vec![ArtifactValidationV1 {
                name: "cargo-lock-structure".into(),
                kind: ArtifactValidationKindV1::Structural,
                required: true,
                parameters: BTreeMap::new(),
            }],
        }
    }

    fn executable_fixture_plan(db: &Trail, source_root: ObjectId) -> ArtifactResolutionPlanV1 {
        let executable = std::env::current_exe().unwrap();
        let entry = db
            .root_file_entry(&source_root, "Cargo.toml")
            .unwrap()
            .unwrap();
        let mut plan = fixture_plan(source_root);
        plan.program = "trail-test-resolver".into();
        plan.resolved_program = executable.to_string_lossy().into_owned();
        plan.executable_identity =
            super::super::workspace_environment::workspace_tool_identity_for_path(&executable)
                .unwrap();
        plan.readable_inputs = vec![ArtifactResolutionInputV1 {
            source_path: "Cargo.toml".into(),
            content_hash: entry.content_hash,
            size_bytes: entry.size_bytes,
        }];
        plan
    }

    fn fixture_candidate(bytes: &[u8]) -> ArtifactResolutionCandidateV1 {
        ArtifactResolutionCandidateV1 {
            snapshot_bytes: bytes.to_vec(),
            resolved_identities: BTreeMap::from([("fixture".into(), "1.0.0".into())]),
            checksums: BTreeMap::from([("fixture".into(), sha256_hex(bytes))]),
            contacted_authorities: vec!["index.crates.io:443".into()],
            stdout: b"resolver completed".to_vec(),
            stderr: Vec::new(),
            redactions: Vec::new(),
        }
    }

    fn initialized_resolution_fixture() -> (tempfile::TempDir, Trail, ObjectId) {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"fixture\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let source_root = db.resolve_refish("main").unwrap().root_id;
        (temp, db, source_root)
    }

    #[test]
    fn malicious_resolution_plans_are_rejected_before_attempt_publication() {
        let (_temp, db, source_root) = initialized_resolution_fixture();
        let base = executable_fixture_plan(&db, source_root);

        let mut traversing_output = base.clone();
        traversing_output.proposal_key = "proposal_traversing_output".into();
        traversing_output.candidate_output = "../Cargo.lock".into();

        let mut absolute_workdir = base.clone();
        absolute_workdir.proposal_key = "proposal_absolute_workdir".into();
        absolute_workdir.working_directory = "/tmp".into();

        let mut control_argument = base.clone();
        control_argument.proposal_key = "proposal_control_argument".into();
        control_argument
            .argv
            .push("--config\ncredential=leak".into());

        let mut excessive_limits = base;
        excessive_limits.proposal_key = "proposal_excessive_limits".into();
        excessive_limits.limits.timeout_ms = MAX_RESOLUTION_TIMEOUT_MS + 1;
        excessive_limits.limits.stdout_bytes = MAX_RESOLUTION_CAPTURE_BYTES + 1;
        excessive_limits.limits.candidate_bytes = MAX_RESOLUTION_CANDIDATE_BYTES + 1;
        excessive_limits.limits.candidate_entries = MAX_RESOLUTION_CANDIDATE_ENTRIES + 1;
        excessive_limits.limits.child_processes = MAX_RESOLUTION_CHILD_PROCESSES + 1;

        for (plan, expected) in [
            (traversing_output, "path must stay inside the workspace"),
            (absolute_workdir, "path must stay inside the workspace"),
            (control_argument, "control characters"),
            (excessive_limits, "exceed host ceilings"),
        ] {
            let error = db.begin_artifact_resolution_attempt(plan).unwrap_err();
            assert!(
                error.to_string().contains(expected),
                "unexpected malicious-plan rejection: {error}"
            );
        }
        assert!(db.artifact_resolution_attempts().unwrap().is_empty());
    }

    fn fixture_desired_material(source_root: ObjectId) -> ArtifactDesiredKeyMaterialV2 {
        ArtifactDesiredKeyMaterialV2 {
            version: 2,
            component_id: "cargo:root".into(),
            adapter_identity: "trail.builtin/cargo@1".into(),
            adapter_implementation_version: "1".into(),
            adapter_distribution_digest: "builtin:cargo:1".into(),
            adapter_protocol: "trail.environment-adapter/builtin-v1".into(),
            resolution_snapshot_id: None,
            source_closure: ArtifactSourceClosureV2 {
                normalizer_version: "source-paths/v1".into(),
                certified_complete: false,
                complete_source_root: Some(source_root),
                declared_inputs: vec![ArtifactResolutionInputV1 {
                    source_path: "Cargo.toml".into(),
                    content_hash: "11".repeat(32),
                    size_bytes: 12,
                }],
            },
            upstream_identities: BTreeMap::new(),
            actions: vec![ArtifactActionIdentityV2 {
                name: "build".into(),
                phase: ArtifactActionPhaseV2::Construct,
                executable_identity: "sha256:cargo".into(),
                argv: vec!["cargo".into(), "build".into(), "--locked".into()],
                working_directory: ".".into(),
                environment_names: vec!["CARGO_TARGET_DIR".into()],
            }],
            outputs: vec![ArtifactOutputContractV2 {
                name: "target".into(),
                output_path: "target".into(),
                mount_path: "target".into(),
                policy: EnvironmentOutputPolicy::ImmutableSeedPrivate,
                reuse: EnvironmentReuseMode::Exact,
                scope: EnvironmentSharingScope::Workspace,
                publish: EnvironmentPublicationTrigger::OnSync,
                gate: None,
            }],
            validations: vec![ArtifactValidationV1 {
                name: "tree".into(),
                kind: ArtifactValidationKindV1::Structural,
                required: true,
                parameters: BTreeMap::new(),
            }],
            source_exports: Vec::new(),
            build_environment: BTreeMap::from([("RUSTFLAGS".into(), "-Cdebuginfo=0".into())]),
            target: "debug".into(),
            platform: "darwin".into(),
            architecture: "aarch64".into(),
            abi: "apple".into(),
            portability_certified: true,
            portability_scope: "workspace".into(),
            trust_scope: "builtin".into(),
            network_policy: "deny".into(),
            script_policy: ArtifactScriptPolicyV1::Deny,
            sandbox_policy: "native-deny-by-default".into(),
        }
    }

    #[test]
    fn desired_key_v2_is_canonical_and_separates_identity_dimensions() {
        let source_root = ObjectId("object_source".into());
        let mut left = fixture_desired_material(source_root.clone());
        left.upstream_identities.insert("z".into(), "2".into());
        left.upstream_identities.insert("a".into(), "1".into());
        let mut right = fixture_desired_material(source_root);
        right.upstream_identities.insert("a".into(), "1".into());
        right.upstream_identities.insert("z".into(), "2".into());
        let left_key = artifact_desired_key_v2(left.clone()).unwrap();
        assert_eq!(left_key, artifact_desired_key_v2(right).unwrap());

        left.resolution_snapshot_id = Some(ObjectId("object_snapshot".into()));
        assert_ne!(left_key, artifact_desired_key_v2(left).unwrap());
    }

    #[test]
    fn desired_key_v2_requires_safe_source_fallback_and_rejects_secrets() {
        let mut material = fixture_desired_material(ObjectId("object_source".into()));
        material.source_closure.complete_source_root = None;
        assert!(artifact_desired_key_v2(material.clone()).is_err());

        material.source_closure.complete_source_root = Some(ObjectId("object_source".into()));
        let first_source = artifact_desired_key_v2(material.clone()).unwrap();
        material.source_closure.complete_source_root = Some(ObjectId("object_other_source".into()));
        assert_ne!(
            first_source,
            artifact_desired_key_v2(material.clone()).unwrap()
        );

        material.portability_certified = false;
        assert!(artifact_desired_key_v2(material.clone()).is_err());
        material.outputs[0].reuse = EnvironmentReuseMode::None;
        material.outputs[0].scope = EnvironmentSharingScope::Lane;
        assert!(artifact_desired_key_v2(material.clone()).is_ok());

        material
            .build_environment
            .insert("API_TOKEN".into(), "super-secret".into());
        assert!(artifact_desired_key_v2(material).is_err());
    }

    #[test]
    fn desired_key_v2_diff_reports_first_and_complete_ordered_invalidation_edges() {
        let previous = fixture_desired_material(ObjectId("object_source".into()));
        let mut current = previous.clone();
        current.resolution_snapshot_id = Some(ObjectId("object_snapshot".into()));
        current.adapter_implementation_version = "2".into();
        current.actions[0].argv.push("--release".into());
        current.outputs[0].mount_path = "target-v2".into();
        current.validations[0]
            .parameters
            .insert("profile".into(), "strict".into());
        current.source_exports.push(ArtifactSourceExportContractV2 {
            name: "bindings".into(),
            output_name: "target".into(),
            artifact_subpath: "generated".into(),
            destination: "src/generated".into(),
            collision_policy: "fail".into(),
            required_validation: "tree".into(),
            required_gate: None,
            authorization_mode: "explicit".into(),
        });
        current.trust_scope = "signed-plugin".into();
        current.sandbox_policy = "native-strict".into();

        let diff = diff_artifact_desired_key_v2(&previous, &current).unwrap();
        assert_ne!(diff.previous_key, diff.current_key);
        assert_eq!(diff.first.as_ref().unwrap().dimension, "resolution");
        assert_eq!(
            diff.edges
                .iter()
                .map(|edge| edge.dimension.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "resolution",
                "tool",
                "action",
                "output",
                "validation",
                "export",
                "trust",
                "sandbox",
            ])
        );
        let mut sorted = diff.edges.clone();
        sorted.sort_by(|left, right| {
            artifact_invalidation_dimension_rank(&left.dimension)
                .cmp(&artifact_invalidation_dimension_rank(&right.dimension))
                .then_with(|| left.name.cmp(&right.name))
                .then_with(|| left.change.cmp(&right.change))
        });
        assert_eq!(diff.edges, sorted);
    }

    #[test]
    fn desired_key_v2_preserves_absence_unicode_and_normalizer_semantics() {
        let material = fixture_desired_material(ObjectId("object_source".into()));
        let mut encoded = serde_json::to_value(&material).unwrap();
        encoded.as_object_mut().unwrap().remove("source_exports");
        assert!(serde_json::from_value::<ArtifactDesiredKeyMaterialV2>(encoded).is_err());

        let mut empty_snapshot = material.clone();
        empty_snapshot.resolution_snapshot_id = Some(ObjectId(String::new()));
        assert!(artifact_desired_key_v2(empty_snapshot).is_err());
        assert!(artifact_desired_key_v2(material.clone()).is_ok());

        let mut composed = material.clone();
        composed.source_closure.certified_complete = true;
        composed.source_closure.complete_source_root = None;
        composed.source_closure.declared_inputs[0].source_path = "café/Cargo.toml".into();
        let composed_key = artifact_desired_key_v2(composed.clone()).unwrap();
        let mut decomposed = composed.clone();
        decomposed.source_closure.declared_inputs[0].source_path = "cafe\u{301}/Cargo.toml".into();
        assert!(artifact_desired_key_v2(decomposed).is_err());

        composed.source_closure.normalizer_version = "source-paths/v2".into();
        assert_ne!(composed_key, artifact_desired_key_v2(composed).unwrap());
    }

    #[test]
    fn each_desired_key_v2_identity_dimension_invalidates_independently() {
        let base = fixture_desired_material(ObjectId("object_source".into()));
        let base_key = artifact_desired_key_v2(base.clone()).unwrap();
        let mut variants = Vec::new();

        let mut value = base.clone();
        value.adapter_distribution_digest = "builtin:cargo:2".into();
        variants.push(("adapter", value));
        let mut value = base.clone();
        value.source_closure.normalizer_version = "source-paths/v2".into();
        variants.push(("source", value));
        let mut value = base.clone();
        value.actions[0].argv.push("--release".into());
        variants.push(("action", value));
        let mut value = base.clone();
        value.outputs[0].mount_path = "target-v2".into();
        variants.push(("output", value));
        let mut value = base.clone();
        value.validations[0].required = false;
        variants.push(("validation", value));
        let mut value = base.clone();
        value.source_exports.push(ArtifactSourceExportContractV2 {
            name: "generated".into(),
            output_name: "target".into(),
            artifact_subpath: "generated".into(),
            destination: "src/generated".into(),
            collision_policy: "fail".into(),
            required_validation: "tree".into(),
            required_gate: None,
            authorization_mode: "explicit".into(),
        });
        variants.push(("export", value));
        let mut value = base.clone();
        value.abi = "musl".into();
        variants.push(("platform", value));
        let mut value = base;
        value.sandbox_policy = "native-strict".into();
        variants.push(("sandbox", value));

        for (dimension, variant) in variants {
            assert_ne!(
                base_key,
                artifact_desired_key_v2(variant).unwrap(),
                "identity dimension `{dimension}` did not invalidate the desired key"
            );
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(32))]

        #[test]
        fn desired_key_v2_is_stable_for_arbitrary_declared_input_order(
            inputs in prop::collection::btree_map("[a-z]{1,12}", any::<u64>(), 0..32)
        ) {
            let mut forward = fixture_desired_material(ObjectId("object_source".into()));
            forward.source_closure.certified_complete = true;
            forward.source_closure.complete_source_root = None;
            forward.source_closure.declared_inputs = inputs
                .iter()
                .map(|(name, value)| ArtifactResolutionInputV1 {
                    source_path: format!("inputs/input-{name}"),
                    content_hash: sha256_hex(&value.to_le_bytes()),
                    size_bytes: 8,
                })
                .collect();
            if forward.source_closure.declared_inputs.is_empty() {
                forward.source_closure.declared_inputs.push(ArtifactResolutionInputV1 {
                    source_path: "inputs/empty".into(),
                    content_hash: sha256_hex(b"empty"),
                    size_bytes: 0,
                });
            }
            let mut reverse = forward.clone();
            reverse.source_closure.declared_inputs.reverse();
            prop_assert_eq!(
                artifact_desired_key_v2(forward).unwrap(),
                artifact_desired_key_v2(reverse).unwrap()
            );
        }
    }

    #[test]
    #[ignore = "qualification benchmark; run explicitly with --nocapture"]
    fn artifact_cas_benchmark_records_whole_chunked_and_successor_reuse() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();

        let whole_bytes = deterministic_benchmark_bytes(ARTIFACT_WHOLE_BLOB_MAX_BYTES);
        let whole_cpu = process_cpu_micros();
        let whole_started = Instant::now();
        let whole_file = db.ingest_artifact_file_bytes(&whole_bytes, 0o644).unwrap();
        let whole_wall_micros = whole_started.elapsed().as_micros() as u64;
        let whole_cpu_micros = process_cpu_micros().saturating_sub(whole_cpu);
        let whole_node: ArtifactFileNodeV1 = db
            .get_artifact_cas_object(
                &whole_file.0,
                ARTIFACT_FILE_NODE_KIND,
                ARTIFACT_FILE_NODE_VERSION,
            )
            .unwrap();
        assert!(matches!(
            whole_node.content,
            ArtifactFileContentV1::Blob { .. }
        ));
        let whole_counts = artifact_benchmark_storage_counts(&db);

        let chunked_bytes = deterministic_benchmark_bytes(16 * 1024 * 1024);
        let chunked_cpu = process_cpu_micros();
        let chunked_started = Instant::now();
        let chunked_file = db
            .ingest_artifact_file_bytes(&chunked_bytes, 0o644)
            .unwrap();
        let chunked_wall_micros = chunked_started.elapsed().as_micros() as u64;
        let chunked_cpu_micros = process_cpu_micros().saturating_sub(chunked_cpu);
        let chunked_node: ArtifactFileNodeV1 = db
            .get_artifact_cas_object(
                &chunked_file.0,
                ARTIFACT_FILE_NODE_KIND,
                ARTIFACT_FILE_NODE_VERSION,
            )
            .unwrap();
        let ArtifactFileContentV1::Chunks { chunk_list_id } = chunked_node.content else {
            panic!("large benchmark file did not use FastCDC chunks");
        };
        let initial_list: ArtifactChunkListV1 = db
            .get_artifact_cas_object(
                &chunk_list_id.0,
                ARTIFACT_CHUNK_LIST_KIND,
                ARTIFACT_CHUNK_LIST_VERSION,
            )
            .unwrap();
        assert!(initial_list.chunks.len() > 1);
        let chunked_counts = artifact_benchmark_storage_counts(&db);

        let mut successor_bytes = chunked_bytes;
        let midpoint = successor_bytes.len() / 2;
        for byte in &mut successor_bytes[midpoint..midpoint + 64] {
            *byte ^= 0x5a;
        }
        let successor_cpu = process_cpu_micros();
        let successor_started = Instant::now();
        let successor_file = db
            .ingest_artifact_file_bytes(&successor_bytes, 0o644)
            .unwrap();
        let successor_wall_micros = successor_started.elapsed().as_micros() as u64;
        let successor_cpu_micros = process_cpu_micros().saturating_sub(successor_cpu);
        let successor_node: ArtifactFileNodeV1 = db
            .get_artifact_cas_object(
                &successor_file.0,
                ARTIFACT_FILE_NODE_KIND,
                ARTIFACT_FILE_NODE_VERSION,
            )
            .unwrap();
        let ArtifactFileContentV1::Chunks {
            chunk_list_id: successor_list_id,
        } = successor_node.content
        else {
            panic!("large successor benchmark file did not use FastCDC chunks");
        };
        let successor_list: ArtifactChunkListV1 = db
            .get_artifact_cas_object(
                &successor_list_id.0,
                ARTIFACT_CHUNK_LIST_KIND,
                ARTIFACT_CHUNK_LIST_VERSION,
            )
            .unwrap();
        let initial_chunks = initial_list
            .chunks
            .iter()
            .map(|chunk| &chunk.chunk_id)
            .collect::<BTreeSet<_>>();
        let reused_chunks = successor_list
            .chunks
            .iter()
            .filter(|chunk| initial_chunks.contains(&chunk.chunk_id))
            .count();
        assert!(reused_chunks > 0, "successor failed to reuse any CDC chunk");
        let successor_counts = artifact_benchmark_storage_counts(&db);

        println!(
            "{}",
            serde_json::json!({
                "schema": "trail.artifact-cas-benchmark/v1",
                "whole": {
                    "logical_bytes": whole_bytes.len(),
                    "cpu_micros": whole_cpu_micros,
                    "wall_micros": whole_wall_micros,
                    "object_count": whole_counts.0,
                    "unique_object_bytes": whole_counts.2,
                },
                "chunked": {
                    "logical_bytes": 16 * 1024 * 1024,
                    "cpu_micros": chunked_cpu_micros,
                    "wall_micros": chunked_wall_micros,
                    "new_object_count": chunked_counts.0 - whole_counts.0,
                    "new_unique_object_bytes": chunked_counts.2 - whole_counts.2,
                    "chunk_count": initial_list.chunks.len(),
                },
                "successor": {
                    "logical_bytes": successor_bytes.len(),
                    "cpu_micros": successor_cpu_micros,
                    "wall_micros": successor_wall_micros,
                    "new_object_count": successor_counts.0 - chunked_counts.0,
                    "new_unique_object_bytes": successor_counts.2 - chunked_counts.2,
                    "chunk_count": successor_list.chunks.len(),
                    "reused_chunks": reused_chunks,
                }
            })
        );
    }

    fn artifact_benchmark_storage_counts(db: &Trail) -> (u64, u64, u64) {
        db.conn
            .query_row(
                "SELECT COUNT(*),COALESCE(SUM(a.logical_bytes),0),
                        COALESCE(SUM(LENGTH(o.bytes)),0)
                 FROM artifact_objects a JOIN objects o ON o.object_id=a.object_id",
                [],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)? as u64,
                        row.get::<_, i64>(1)? as u64,
                        row.get::<_, i64>(2)? as u64,
                    ))
                },
            )
            .unwrap()
    }

    fn deterministic_benchmark_bytes(size: usize) -> Vec<u8> {
        let mut state = 0x4d59_5df4_d0f3_3173_u64;
        (0..size)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    }

    #[cfg(unix)]
    fn process_cpu_micros() -> u64 {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::uninit();
        // SAFETY: getrusage initializes the supplied rusage on success, and
        // the pointer is valid for the duration of this call.
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } != 0 {
            return 0;
        }
        // SAFETY: the success branch above guarantees initialization.
        let usage = unsafe { usage.assume_init() };
        timeval_micros(usage.ru_utime).saturating_add(timeval_micros(usage.ru_stime))
    }

    #[cfg(not(unix))]
    fn process_cpu_micros() -> u64 {
        0
    }

    #[cfg(unix)]
    fn timeval_micros(value: libc::timeval) -> u64 {
        u64::try_from(value.tv_sec)
            .unwrap_or(0)
            .saturating_mul(1_000_000)
            .saturating_add(u64::try_from(value.tv_usec).unwrap_or(0))
    }

    #[test]
    fn divergent_tree_roots_for_one_desired_key_are_quarantined_and_held() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(workspace.path().join("README.md"), "root\n").unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let first_source = tempfile::tempdir().unwrap();
        let second_source = tempfile::tempdir().unwrap();
        fs::write(first_source.path().join("result"), "first\n").unwrap();
        fs::write(second_source.path().join("result"), "second\n").unwrap();
        let (first_tree, _) = db
            .ingest_artifact_tree_under_write_lock(first_source.path())
            .unwrap();
        let (second_tree, _) = db
            .ingest_artifact_tree_under_write_lock(second_source.path())
            .unwrap();
        let desired_key =
            artifact_desired_key_v2(fixture_desired_material(ObjectId("object_source".into())))
                .unwrap();
        let envelope = |tree_root_id| ArtifactEnvelopeV1 {
            version: ARTIFACT_ENVELOPE_VERSION,
            desired_identity: ArtifactDesiredIdentityV1::ArtifactDesiredV2 {
                desired_key: desired_key.clone(),
            },
            tree_root_id,
            component_id: "cargo:root".into(),
            output_name: "target".into(),
            output_policy: EnvironmentOutputPolicy::ImmutableSeedPrivate,
            portability_scope: "workspace".into(),
            trust_scope: "builtin".into(),
            secret_taint: ArtifactSecretTaintV1::Clear,
            resolution_snapshot_id: None,
            validation_receipt_ids: Vec::new(),
        };

        let (first_envelope, first_quarantined) = db
            .put_artifact_envelope_under_write_lock(envelope(first_tree.clone()))
            .unwrap();
        assert!(!first_quarantined);
        assert_eq!(
            db.artifact_envelope_ids().unwrap(),
            vec![first_envelope.0.clone()]
        );
        db.verify_ready_artifact_envelope_under_write_lock(&first_envelope, &first_tree)
            .unwrap();
        let inspection = db.inspect_artifact(&first_envelope).unwrap();
        assert_eq!(inspection.state, "ready");
        assert_eq!(inspection.verification_state, "verified");
        assert_eq!(inspection.trust_state, "trusted");
        assert_eq!(inspection.quarantine_state, "none");
        assert_eq!(inspection.tree_root_id, first_tree);
        assert!(inspection.reachability.complete);
        assert!(inspection.reachability.object_count >= 4);
        assert!(inspection.storage.logical_bytes > 0);
        assert!(
            db.verify_artifact(&first_envelope, ArtifactVerificationLevelV1::Full)
                .unwrap()
                .valid
        );
        let reproduce = db
            .verify_artifact(&first_envelope, ArtifactVerificationLevelV1::Reproduce)
            .unwrap();
        assert!(!reproduce.valid);
        assert_eq!(reproduce.reproduction_evidence_valid, Some(false));
        let space = db.workspace_artifact_space().unwrap();
        assert_eq!(space.envelope_count, 1);
        assert_eq!(space.active_quarantine_count, 0);
        assert!(space.storage.logical_bytes > 0);
        let (second_envelope, second_quarantined) = db
            .put_artifact_envelope_under_write_lock(envelope(second_tree.clone()))
            .unwrap();
        assert!(second_quarantined);
        assert!(db
            .verify_ready_artifact_envelope_under_write_lock(&first_envelope, &first_tree)
            .is_err());
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM artifact_envelopes
                     WHERE envelope_id IN (?1,?2) AND state='quarantined'",
                    params![first_envelope.0, second_envelope.0],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );
        let (quarantine_id, evidence_object_id) = db
            .conn
            .query_row(
                "SELECT quarantine_id,evidence_object_id FROM artifact_quarantines
                 WHERE desired_key=?1 AND trust_scope='builtin' AND state='active'",
                params![desired_key.0],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .unwrap();
        assert!(quarantine_id.starts_with("artifact_quarantine_"));
        let evidence: ArtifactDivergenceEvidenceV1 = db
            .get_object(
                ARTIFACT_DIVERGENCE_EVIDENCE_KIND,
                &ObjectId(evidence_object_id),
            )
            .unwrap();
        assert_eq!(evidence.incumbent_tree_root_id, first_tree);
        assert_eq!(evidence.candidate_tree_root_id, second_tree);
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM artifact_holds WHERE reason=?1",
                    params![quarantine_id],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            2
        );
        let quarantine_id = ArtifactQuarantineId::parse(quarantine_id).unwrap();
        let quarantine_list = db.artifact_quarantine_list_report().unwrap();
        assert_eq!(quarantine_list.active_count, 1);
        assert_eq!(quarantine_list.resolved_count, 0);
        assert_eq!(quarantine_list.quarantines.len(), 1);
        assert_eq!(
            db.artifact_quarantine(&quarantine_id).unwrap().state,
            "active"
        );
        let quarantined_inspection = db.inspect_artifact(&first_envelope).unwrap();
        assert_eq!(quarantined_inspection.state, "quarantined");
        assert_eq!(quarantined_inspection.quarantine_state, "active");
        assert!(
            !db.verify_artifact(&first_envelope, ArtifactVerificationLevelV1::Attach)
                .unwrap()
                .valid
        );
        let resolution_report = db
            .resolve_artifact_quarantine_report(
                &quarantine_id,
                ArtifactQuarantineResolutionV1::RetainPrivate,
            )
            .unwrap();
        let resolved = resolution_report.quarantine;
        assert_eq!(resolved.state, "resolved");
        assert_eq!(resolved.resolution.as_deref(), Some("retain_private"));
        assert_eq!(resolution_report.affected_envelopes.len(), 2);
        assert!(db
            .verify_ready_artifact_envelope_under_write_lock(&second_envelope, &second_tree)
            .is_err());
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM artifact_holds WHERE reason=?1",
                    params![quarantine_id.0],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn legacy_workspace_layer_identity_never_defaults_to_v2() {
        let legacy = ArtifactDesiredIdentityV1::WorkspaceLayerV1 {
            cache_key: "legacy-cache-key".into(),
            canonical_key: WorkspaceLayerKeyV1 {
                kind: "dependency".into(),
                adapter: "node".into(),
                adapter_version: 1,
                inputs: BTreeMap::from([("lock".into(), "digest".into())]),
                tool_versions: BTreeMap::from([("node".into(), "22".into())]),
                platform: "darwin".into(),
                architecture: "aarch64".into(),
                portability_scope: "platform".into(),
                strategy: "npm-ci".into(),
            },
        };
        let encoded = serde_json::to_value(&legacy).unwrap();
        assert_eq!(encoded["identity_version"], "workspace_layer_v1");
        let decoded: ArtifactDesiredIdentityV1 = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, legacy);
        assert!(decoded.desired_key_v2().is_none());
    }

    #[test]
    fn artifact_object_codecs_are_canonical_and_validate_edges() {
        let blob = ArtifactBlobV1 {
            version: ARTIFACT_BLOB_VERSION,
            content_sha256: sha256_hex(b"same bytes"),
            bytes: b"same bytes".to_vec(),
        };
        let (blob_id, blob_bytes) = encode_artifact_blob(blob.clone()).unwrap();
        assert_eq!(decode_artifact_blob(&blob_bytes, &blob_id).unwrap(), blob);

        let file = ArtifactFileNodeV1 {
            version: ARTIFACT_FILE_NODE_VERSION,
            mode: 0o644,
            executable: false,
            size_bytes: 10,
            content_sha256: sha256_hex(b"same bytes"),
            content: ArtifactFileContentV1::Blob { blob_id },
        };
        let (file_id, _) = encode_artifact_file_node(file).unwrap();
        let directory = ArtifactDirectoryNodeV1 {
            version: ARTIFACT_DIRECTORY_NODE_VERSION,
            entries: vec![
                ArtifactDirectoryEntryV1 {
                    name: "z".into(),
                    target: ArtifactDirectoryEntryTargetV1::Symlink { target: "a".into() },
                },
                ArtifactDirectoryEntryV1 {
                    name: "a".into(),
                    target: ArtifactDirectoryEntryTargetV1::File { node_id: file_id },
                },
            ],
        };
        let (directory_id, canonical) = encode_artifact_directory_node(directory).unwrap();
        let decoded = decode_artifact_directory_node(&canonical).unwrap();
        assert_eq!(decoded.entries[0].name, "a");
        assert_eq!(
            directory_id,
            encode_artifact_directory_node(decoded).unwrap().0
        );

        let root = ArtifactTreeRootV1 {
            version: ARTIFACT_TREE_ROOT_VERSION,
            root_directory_id: directory_id.clone(),
            logical_bytes: 10,
            entry_count: 2,
            path_normalizer: "trail-paths/v1".into(),
        };
        assert_ne!(encode_artifact_tree_root(root).unwrap().0, directory_id);
    }

    #[test]
    fn artifact_object_codecs_reject_noncanonical_or_broken_edges() {
        let invalid_directory = ArtifactDirectoryNodeV1 {
            version: ARTIFACT_DIRECTORY_NODE_VERSION,
            entries: vec![
                ArtifactDirectoryEntryV1 {
                    name: "duplicate".into(),
                    target: ArtifactDirectoryEntryTargetV1::Symlink { target: "a".into() },
                },
                ArtifactDirectoryEntryV1 {
                    name: "duplicate".into(),
                    target: ArtifactDirectoryEntryTargetV1::Symlink { target: "b".into() },
                },
            ],
        };
        assert!(encode_artifact_directory_node(invalid_directory).is_err());

        let invalid_chunks = ArtifactChunkListV1 {
            version: ARTIFACT_CHUNK_LIST_VERSION,
            algorithm: "fastcdc-v1".into(),
            file_size_bytes: 99,
            file_sha256: "11".repeat(32),
            chunks: vec![ArtifactChunkRefV1 {
                chunk_id: ArtifactChunkId::new(b"chunk"),
                size_bytes: 1,
            }],
        };
        assert!(encode_artifact_chunk_list(invalid_chunks).is_err());

        let chunk = ArtifactChunkV1 {
            version: ARTIFACT_CHUNK_VERSION,
            content_sha256: "00".repeat(32),
            bytes: b"not zero".to_vec(),
        };
        assert!(encode_artifact_chunk(chunk).is_err());
    }

    #[test]
    fn validation_receipts_are_deterministic_typed_and_bound_to_the_exact_envelope() {
        let (_workspace, mut db, source_root) = initialized_resolution_fixture();
        let candidate = tempfile::tempdir().unwrap();
        fs::write(
            candidate.path().join("validated.bin"),
            b"validated output\n",
        )
        .unwrap();
        let (tree_id, _) = db.ingest_artifact_tree(candidate.path()).unwrap();
        let desired_key = artifact_desired_key_v2(fixture_desired_material(source_root)).unwrap();
        let desired_identity = ArtifactDesiredIdentityV1::ArtifactDesiredV2 {
            desired_key: desired_key.clone(),
        };
        let receipt = ArtifactValidationReceiptV1 {
            version: ARTIFACT_VALIDATION_RECEIPT_VERSION,
            declaration: ArtifactValidationV1 {
                name: "cargo-metadata-loads".into(),
                kind: ArtifactValidationKindV1::Loadability,
                required: true,
                parameters: BTreeMap::from([("format".into(), "cargo-metadata-v1".into())]),
            },
            desired_identity: desired_identity.clone(),
            tree_root_id: tree_id.clone(),
            validator_identity: "trail.builtin/cargo-validator@1#sha256:fixture".into(),
            validated_input_digest: sha256_hex(b"desired+tree+validator+policy"),
            outcome: ArtifactValidationOutcomeV1::Passed,
            evidence: BTreeMap::from([
                ("checked_entries".into(), "1".into()),
                ("result".into(), "loadable".into()),
            ]),
        };
        let receipt_id = db.put_artifact_validation_receipt(receipt.clone()).unwrap();
        assert_eq!(
            db.put_artifact_validation_receipt(receipt.clone()).unwrap(),
            receipt_id
        );
        assert_eq!(
            db.artifact_validation_receipt(&receipt_id).unwrap(),
            receipt
        );

        let envelope = ArtifactEnvelopeV1 {
            version: ARTIFACT_ENVELOPE_VERSION,
            desired_identity: desired_identity.clone(),
            tree_root_id: tree_id.clone(),
            component_id: "cargo:root".into(),
            output_name: "target".into(),
            output_policy: EnvironmentOutputPolicy::ImmutableSeedPrivate,
            portability_scope: "workspace".into(),
            trust_scope: "builtin".into(),
            secret_taint: ArtifactSecretTaintV1::Clear,
            resolution_snapshot_id: None,
            validation_receipt_ids: vec![receipt_id.clone()],
        };
        let (envelope_id, quarantined) = db
            .put_artifact_envelope_under_write_lock(envelope.clone())
            .unwrap();
        assert!(!quarantined);
        let attestations = db.artifact_attestations_for_envelope(&envelope_id).unwrap();
        assert_eq!(attestations.len(), 1);
        assert_eq!(
            attestations[0].attestation.statement.envelope_id,
            envelope_id
        );
        assert_eq!(attestations[0].attestation.statement.tree_root_id, tree_id);
        assert_eq!(attestations[0].attestation.signature, None);
        let verification = db
            .verify_artifact_attestation(&attestations[0].attestation_id)
            .unwrap();
        assert!(verification.valid);
        assert_eq!(verification.signature_status, "unsigned");
        let mut invalid_signed = attestations[0].attestation.clone();
        invalid_signed.signature = Some(ArtifactAttestationSignatureV1 {
            algorithm: "ed25519".into(),
            key_id: "fixture-key".into(),
            public_key_hex: "00".repeat(32),
            signature_hex: "00".repeat(64),
        });
        let mut signature_diagnostics = Vec::new();
        let (_, signature_valid) = db
            .verify_artifact_attestation_signature(&invalid_signed, &mut signature_diagnostics)
            .unwrap();
        assert!(!signature_valid);
        assert!(!signature_diagnostics.is_empty());
        let (same_envelope_id, _) = db
            .put_artifact_envelope_under_write_lock(envelope.clone())
            .unwrap();
        assert_eq!(same_envelope_id, envelope_id);
        assert_eq!(
            db.artifact_attestations_for_envelope(&envelope_id)
                .unwrap()
                .len(),
            1
        );
        db.conn
            .execute(
                "UPDATE artifact_attestations SET state='revoked' WHERE attestation_id=?1",
                params![&attestations[0].attestation_id.0],
            )
            .unwrap();
        assert!(
            !db.verify_artifact_attestation(&attestations[0].attestation_id)
                .unwrap()
                .valid
        );
        assert!(db
            .verify_ready_artifact_envelope_under_write_lock(&envelope_id, &tree_id)
            .unwrap_err()
            .to_string()
            .contains("database state is `revoked`"));
        db.conn
            .execute(
                "UPDATE artifact_attestations SET state='valid' WHERE attestation_id=?1",
                params![&attestations[0].attestation_id.0],
            )
            .unwrap();
        assert_eq!(
            db.verify_ready_artifact_envelope_under_write_lock(&envelope_id, &tree_id)
                .unwrap()
                .validation_receipt_ids,
            vec![receipt_id.clone()]
        );
        assert!(db.validate_artifact_cas_integrity().unwrap().is_empty());

        let mut tainted_envelope = envelope.clone();
        tainted_envelope.secret_taint = ArtifactSecretTaintV1::Tainted {
            channels: vec!["runtime_credential".into()],
        };
        let error = db
            .put_artifact_envelope_under_write_lock(tainted_envelope)
            .unwrap_err();
        assert!(error.to_string().contains("must remain lane-private"));
        db.conn
            .execute(
                "INSERT INTO artifact_holds(
                     hold_id,target_kind,target_id,reason,created_at)
                 VALUES('hold_validation_receipt','artifact_envelope',?1,'validation-test',1)",
                params![envelope_id.0],
            )
            .unwrap();
        db.gc(false).unwrap();
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM objects WHERE object_id=?1",
                    params![receipt_id.0],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            1
        );

        let mut failed_receipt = receipt.clone();
        failed_receipt.outcome = ArtifactValidationOutcomeV1::Failed;
        let failed_id = db.put_artifact_validation_receipt(failed_receipt).unwrap();
        let mut failed_envelope = envelope.clone();
        failed_envelope.validation_receipt_ids = vec![failed_id];
        assert!(db
            .put_artifact_envelope_under_write_lock(failed_envelope)
            .unwrap_err()
            .to_string()
            .contains("does not pass"));

        let mut secret_receipt = receipt;
        secret_receipt.evidence.insert(
            "output".into(),
            "Authorization: Bearer validator-secret".into(),
        );
        assert!(db
            .put_artifact_validation_receipt(secret_receipt)
            .unwrap_err()
            .to_string()
            .contains("secret material"));
    }

    #[test]
    fn attachment_fails_closed_for_tampered_attestation_references_and_bytes() {
        let (workspace, db, source_root) = initialized_resolution_fixture();
        let candidate = tempfile::tempdir().unwrap();
        fs::write(candidate.path().join("artifact.bin"), b"attested output\n").unwrap();
        let (tree_id, _) = db.ingest_artifact_tree(candidate.path()).unwrap();
        let desired_key = artifact_desired_key_v2(fixture_desired_material(source_root)).unwrap();
        let (envelope_id, quarantined) = db
            .put_artifact_envelope_under_write_lock(ArtifactEnvelopeV1 {
                version: ARTIFACT_ENVELOPE_VERSION,
                desired_identity: ArtifactDesiredIdentityV1::ArtifactDesiredV2 { desired_key },
                tree_root_id: tree_id.clone(),
                component_id: "cargo:root".into(),
                output_name: "target".into(),
                output_policy: EnvironmentOutputPolicy::ImmutableSeedPrivate,
                portability_scope: "workspace".into(),
                trust_scope: "builtin".into(),
                secret_taint: ArtifactSecretTaintV1::Clear,
                resolution_snapshot_id: None,
                validation_receipt_ids: Vec::new(),
            })
            .unwrap();
        assert!(!quarantined);
        let attestation = db
            .artifact_attestations_for_envelope(&envelope_id)
            .unwrap()
            .pop()
            .unwrap();

        db.conn
            .execute(
                "UPDATE artifact_attestations SET producer_identity='tampered-producer'
                 WHERE attestation_id=?1",
                params![attestation.attestation_id.0],
            )
            .unwrap();
        let reference_error = db
            .verify_ready_artifact_envelope_under_write_lock(&envelope_id, &tree_id)
            .unwrap_err();
        assert!(reference_error
            .to_string()
            .contains("database identity disagrees"));

        db.conn
            .execute(
                "UPDATE artifact_attestations SET producer_identity=?1
                 WHERE attestation_id=?2",
                params![
                    attestation.attestation.statement.producer_identity,
                    attestation.attestation_id.0
                ],
            )
            .unwrap();
        db.verify_ready_artifact_envelope_under_write_lock(&envelope_id, &tree_id)
            .unwrap();
        db.conn
            .execute(
                "UPDATE objects SET bytes=X'00' WHERE object_id=?1",
                params![attestation.object_id.0],
            )
            .unwrap();
        drop(db);

        let reopened = Trail::open(workspace.path()).unwrap();
        let byte_error = reopened
            .verify_ready_artifact_envelope_under_write_lock(&envelope_id, &tree_id)
            .unwrap_err();
        assert!(
            byte_error.to_string().contains("attestation")
                || byte_error.to_string().contains("object")
                || byte_error.to_string().contains("CBOR")
                || byte_error.to_string().contains("serialization error"),
            "unexpected tampered-attestation rejection: {byte_error}"
        );
    }

    #[test]
    fn artifact_file_storage_uses_whole_blobs_then_fastcdc_chunks() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let _lock = db.acquire_write_lock().unwrap();

        let small = vec![b'a'; ARTIFACT_WHOLE_BLOB_MAX_BYTES];
        let small_id = db.ingest_artifact_file_bytes(&small, 0o644).unwrap();
        let small_file: ArtifactFileNodeV1 = db
            .get_object(
                ARTIFACT_FILE_NODE_KIND,
                &artifact_object_id(&db, &small_id.0),
            )
            .unwrap();
        assert!(matches!(
            small_file.content,
            ArtifactFileContentV1::Blob { .. }
        ));

        let mut large = Vec::with_capacity(5 * 1024 * 1024);
        for index in 0..5 * 1024 * 1024 {
            large.push(((index * 31 + index / 97) % 251) as u8);
        }
        let large_id = db.ingest_artifact_file_bytes(&large, 0o755).unwrap();
        let large_file: ArtifactFileNodeV1 = db
            .get_object(
                ARTIFACT_FILE_NODE_KIND,
                &artifact_object_id(&db, &large_id.0),
            )
            .unwrap();
        let ArtifactFileContentV1::Chunks { chunk_list_id } = large_file.content else {
            panic!("large artifact file must use chunks");
        };
        let chunk_list: ArtifactChunkListV1 = db
            .get_object(
                ARTIFACT_CHUNK_LIST_KIND,
                &artifact_object_id(&db, &chunk_list_id.0),
            )
            .unwrap();
        assert_eq!(chunk_list.algorithm, "fastcdc-v1");
        assert_eq!(chunk_list.file_size_bytes, large.len() as u64);
        assert!(chunk_list.chunks.len() >= 2);
        assert!(chunk_list
            .chunks
            .iter()
            .all(|chunk| chunk.size_bytes <= ARTIFACT_CHUNK_MAX_BYTES as u64));
    }

    #[test]
    fn artifact_manifest_lazy_lookup_and_ranged_reads_do_not_require_full_tree() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::create_dir_all(source.path().join("nested")).unwrap();
        let large = deterministic_benchmark_bytes(6 * 1024 * 1024);
        fs::write(source.path().join("nested/large.bin"), &large).unwrap();
        fs::write(source.path().join("small.txt"), b"small artifact\n").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink("../small.txt", source.path().join("nested/link")).unwrap();
        let (tree_id, _) = db.ingest_artifact_tree(source.path()).unwrap();

        assert!(matches!(
            db.artifact_tree_lazy_entry(&tree_id, "nested").unwrap(),
            Some(ArtifactLazyEntry::Directory { .. })
        ));
        let Some(ArtifactLazyEntry::File {
            node_id,
            mode,
            size_bytes,
        }) = db
            .artifact_tree_lazy_entry(&tree_id, "nested/large.bin")
            .unwrap()
        else {
            panic!("large artifact path must resolve lazily");
        };
        assert_eq!(mode, 0o644);
        assert_eq!(size_bytes, large.len() as u64);
        assert!(db
            .artifact_tree_lazy_entry(&tree_id, "nested/missing")
            .unwrap()
            .is_none());
        let children = db.artifact_tree_lazy_children(&tree_id, "nested").unwrap();
        assert!(children.iter().any(|(name, _)| name == "large.bin"));
        #[cfg(unix)]
        assert!(children.iter().any(|(name, entry)| {
            name == "link" && matches!(entry, ArtifactLazyEntry::Symlink { .. })
        }));

        let file = db.verified_artifact_file(&node_id).unwrap();
        let ArtifactFileContentV1::Chunks { chunk_list_id } = file.content else {
            panic!("large lazy-read fixture must be chunked");
        };
        let list: ArtifactChunkListV1 = db
            .get_artifact_cas_object(
                &chunk_list_id.0,
                ARTIFACT_CHUNK_LIST_KIND,
                ARTIFACT_CHUNK_LIST_VERSION,
            )
            .unwrap();
        assert!(list.chunks.len() > 1);
        let first_size = list.chunks[0].size_bytes;
        let last = list.chunks.last().unwrap().chunk_id.clone();
        db.conn
            .execute(
                "UPDATE artifact_objects SET kind='intentionally-unavailable-test-chunk' \
                 WHERE artifact_id=?1",
                params![last.0],
            )
            .unwrap();
        let count = u32::try_from(first_size.min(64 * 1024)).unwrap();
        assert_eq!(
            db.artifact_file_read_range(&node_id, 0, count).unwrap(),
            large[..count as usize]
        );
    }

    #[test]
    fn artifact_file_materialization_never_removes_a_preexisting_destination() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("artifact.txt"), b"artifact bytes\n").unwrap();
        let (tree_id, _) = db.ingest_artifact_tree(source.path()).unwrap();
        let Some(ArtifactLazyEntry::File { node_id, .. }) = db
            .artifact_tree_lazy_entry(&tree_id, "artifact.txt")
            .unwrap()
        else {
            panic!("artifact fixture must resolve to a file");
        };
        let destination_root = tempfile::tempdir().unwrap();
        let destination = destination_root.path().join("existing.txt");
        fs::write(&destination, b"user-owned bytes\n").unwrap();

        assert!(db
            .materialize_artifact_file(&node_id, &destination)
            .is_err());
        assert_eq!(fs::read(destination).unwrap(), b"user-owned bytes\n");
    }

    #[test]
    fn equal_artifact_files_reuse_cas_objects_across_desired_keys() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let _lock = db.acquire_write_lock().unwrap();
        let bytes = b"framework-neutral reusable bytes";
        let first = db.ingest_artifact_file_bytes(bytes, 0o644).unwrap();
        let object_count = db
            .conn
            .query_row("SELECT COUNT(*) FROM artifact_objects", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let second = db.ingest_artifact_file_bytes(bytes, 0o644).unwrap();
        assert_eq!(first, second);
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM artifact_objects", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            object_count
        );
    }

    #[test]
    fn artifact_tree_ingestion_is_order_independent_and_reuses_content() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let left = tempfile::tempdir().unwrap();
        let right = tempfile::tempdir().unwrap();
        for root in [left.path(), right.path()] {
            fs::create_dir_all(root.join("nested")).unwrap();
        }
        fs::write(left.path().join("z.txt"), "z\n").unwrap();
        fs::write(left.path().join("nested/a.txt"), "a\n").unwrap();
        fs::write(right.path().join("nested/a.txt"), "a\n").unwrap();
        fs::write(right.path().join("z.txt"), "z\n").unwrap();

        let (left_id, left_tree) = db.ingest_artifact_tree(left.path()).unwrap();
        let object_count = db
            .conn
            .query_row("SELECT COUNT(*) FROM artifact_objects", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        let (right_id, right_tree) = db.ingest_artifact_tree(right.path()).unwrap();
        assert_eq!(left_id, right_id);
        assert_eq!(left_tree, right_tree);
        assert_eq!(left_tree.entry_count, 3);
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM artifact_objects", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            object_count
        );
    }

    #[cfg(unix)]
    #[test]
    fn artifact_tree_ingestion_rejects_escaping_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let source = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink("../outside", source.path().join("escape")).unwrap();
        let error = db.ingest_artifact_tree(source.path()).unwrap_err();
        assert!(error.to_string().contains("escapes"));
    }

    #[test]
    fn artifact_tree_ingestion_rejects_secret_content() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(
            source.path().join("generated.env"),
            "API_TOKEN=do-not-store\n",
        )
        .unwrap();
        let error = db.ingest_artifact_tree(source.path()).unwrap_err();
        assert!(error.to_string().contains("secret material"));
        assert!(error.to_string().contains("generated.env"));
    }

    #[test]
    fn artifact_tree_secret_policy_allows_dependency_source_literals() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(
            source.path().join("client.js"),
            "export const example = 'Authorization: Bearer abc123';\n",
        )
        .unwrap();

        db.ingest_artifact_tree(source.path()).unwrap();

        fs::write(
            source.path().join("private.pem"),
            "-----BEGIN PRIVATE KEY-----\nkey-material\n-----END PRIVATE KEY-----\n",
        )
        .unwrap();
        let error = db.ingest_artifact_tree(source.path()).unwrap_err();
        assert!(error.to_string().contains("private.pem"));
        assert!(error.to_string().contains("secret material"));

        let public_type_fixture = b"export type Example = '-----BEGIN PRIVATE KEY-----';\n";
        validate_artifact_secret_policy(
            public_type_fixture,
            Some("@example/openapi-types/types.d.ts"),
            ArtifactSecretPolicy::LockedPublicDependencies,
        )
        .unwrap();
        let error = validate_artifact_secret_policy(
            public_type_fixture,
            Some("package/private.key"),
            ArtifactSecretPolicy::LockedPublicDependencies,
        )
        .unwrap_err();
        assert!(error.to_string().contains("private.key"));
    }

    #[cfg(unix)]
    #[test]
    fn artifact_tree_ingestion_rejects_privileged_modes_and_xattrs() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();

        let privileged = tempfile::tempdir().unwrap();
        let privileged_file = privileged.path().join("tool");
        fs::write(&privileged_file, "tool\n").unwrap();
        let mut permissions = fs::metadata(&privileged_file).unwrap().permissions();
        permissions.set_mode(0o4755);
        fs::set_permissions(&privileged_file, permissions).unwrap();
        let error = db.ingest_artifact_tree(privileged.path()).unwrap_err();
        assert!(error.to_string().contains("setuid"));

        let attributed = tempfile::tempdir().unwrap();
        let attributed_file = attributed.path().join("artifact");
        fs::write(&attributed_file, "clean\n").unwrap();
        let attribute = if cfg!(target_os = "macos") {
            "com.trail.test"
        } else {
            "user.trail-test"
        };
        xattr::set(&attributed_file, attribute, b"value").unwrap();
        let error = db.ingest_artifact_tree(attributed.path()).unwrap_err();
        assert!(error.to_string().contains("extended attribute"));
    }

    #[test]
    fn artifact_object_publication_rolls_back_and_retries_after_interruption() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let _lock = db.acquire_write_lock().unwrap();
        db.conn
            .execute_batch(
                "CREATE TRIGGER fail_artifact_mapping
                 BEFORE INSERT ON artifact_objects
                 BEGIN SELECT RAISE(ABORT, 'injected artifact publication failure'); END;",
            )
            .unwrap();
        assert!(db
            .ingest_artifact_file_bytes(b"atomic bytes", 0o644)
            .is_err());
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM objects WHERE kind IN
                     ('ArtifactBlob','ArtifactFileNode')",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .unwrap(),
            0
        );
        db.conn
            .execute_batch("DROP TRIGGER fail_artifact_mapping")
            .unwrap();
        db.ingest_artifact_file_bytes(b"atomic bytes", 0o644)
            .unwrap();
    }

    #[test]
    fn artifact_object_publication_detects_corrupt_preexisting_bytes() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let _lock = db.acquire_write_lock().unwrap();
        let file_id = db
            .ingest_artifact_file_bytes(b"collision evidence", 0o644)
            .unwrap();
        let file: ArtifactFileNodeV1 = db
            .get_object(
                ARTIFACT_FILE_NODE_KIND,
                &artifact_object_id(&db, &file_id.0),
            )
            .unwrap();
        let ArtifactFileContentV1::Blob { blob_id } = file.content else {
            panic!("small fixture must use a whole blob");
        };
        db.conn
            .execute(
                "UPDATE objects SET bytes=x'00' WHERE object_id=?1",
                params![artifact_object_id(&db, &blob_id.0).0],
            )
            .unwrap();
        let error = db
            .ingest_artifact_file_bytes(b"collision evidence", 0o644)
            .unwrap_err();
        assert!(error.to_string().contains("conflicts"));
    }

    #[test]
    fn concurrent_equal_tree_publishers_converge_and_reopen() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("artifact"), "shared\n").unwrap();
        let source = source.path().to_path_buf();
        let workspace = temp.path().to_path_buf();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let mut handles = Vec::new();
        for _ in 0..2 {
            let source = source.clone();
            let workspace = workspace.clone();
            let barrier = barrier.clone();
            handles.push(std::thread::spawn(move || {
                let db = Trail::open(workspace).unwrap();
                barrier.wait();
                Trail::with_write_lock_wait(Duration::from_secs(10), || {
                    db.ingest_artifact_tree(&source)
                })
                .unwrap()
                .0
            }));
        }
        let first = handles.remove(0).join().unwrap();
        let second = handles.remove(0).join().unwrap();
        assert_eq!(first, second);

        let reopened = Trail::open(temp.path()).unwrap();
        assert_eq!(
            artifact_object_id(&reopened, &first.0),
            artifact_object_id(&reopened, &second.0)
        );
    }

    #[test]
    fn successor_large_file_reuses_unchanged_fastcdc_chunks() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let _lock = db.acquire_write_lock().unwrap();
        let mut first_bytes = Vec::with_capacity(12 * 1024 * 1024);
        for index in 0..12 * 1024 * 1024 {
            first_bytes.push(((index * 17 + index / 53 + index / 997) % 251) as u8);
        }
        let first = db.ingest_artifact_file_bytes(&first_bytes, 0o644).unwrap();
        let mut second_bytes = first_bytes;
        second_bytes[6 * 1024 * 1024] ^= 0x5a;
        let second = db.ingest_artifact_file_bytes(&second_bytes, 0o644).unwrap();
        let first_chunks = artifact_file_chunk_ids(&db, &first);
        let second_chunks = artifact_file_chunk_ids(&db, &second);
        assert!(first_chunks.len() >= 3);
        assert!(
            first_chunks.intersection(&second_chunks).next().is_some(),
            "a small successor edit should preserve at least one content-defined chunk"
        );
    }

    #[cfg(unix)]
    #[test]
    fn artifact_tree_normalizes_hardlinks_and_confined_symlinks() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("original"), "same inode\n").unwrap();
        fs::hard_link(
            source.path().join("original"),
            source.path().join("hardlink"),
        )
        .unwrap();
        std::os::unix::fs::symlink("original", source.path().join("symlink")).unwrap();
        let (_, tree) = db.ingest_artifact_tree(source.path()).unwrap();
        assert_eq!(tree.entry_count, 3);
        let root: ArtifactDirectoryNodeV1 = db
            .get_object(
                ARTIFACT_DIRECTORY_NODE_KIND,
                &artifact_object_id(&db, &tree.root_directory_id.0),
            )
            .unwrap();
        let file_ids = root
            .entries
            .iter()
            .filter_map(|entry| match &entry.target {
                ArtifactDirectoryEntryTargetV1::File { node_id } => Some(node_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(file_ids.len(), 2);
        assert_eq!(file_ids[0], file_ids[1]);
        assert!(root.entries.iter().any(|entry| matches!(
            &entry.target,
            ArtifactDirectoryEntryTargetV1::Symlink { target } if target == "original"
        )));
    }

    #[test]
    fn artifact_tree_rejects_non_nfc_and_case_colliding_paths() {
        let non_nfc = ArtifactDirectoryNodeV1 {
            version: ARTIFACT_DIRECTORY_NODE_VERSION,
            entries: vec![ArtifactDirectoryEntryV1 {
                name: "e\u{301}".into(),
                target: ArtifactDirectoryEntryTargetV1::Symlink {
                    target: "safe".into(),
                },
            }],
        };
        assert!(encode_artifact_directory_node(non_nfc).is_err());
        assert!(
            validate_no_case_fold_collisions(&["Node".to_string(), "node".to_string()]).is_err()
        );
    }

    #[test]
    fn artifact_tree_materializes_from_authoritative_objects() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::create_dir(source.path().join("nested")).unwrap();
        fs::write(source.path().join("nested/artifact"), "reconstruct me\n").unwrap();
        let (tree_id, _) = db.ingest_artifact_tree(source.path()).unwrap();
        let materialization_parent = tempfile::tempdir().unwrap();
        let destination = materialization_parent.path().join("materialized");
        let _lock = db.acquire_write_lock().unwrap();
        db.materialize_artifact_tree_under_write_lock(&tree_id, &destination)
            .unwrap();
        assert_eq!(
            fs::read(destination.join("nested/artifact")).unwrap(),
            b"reconstruct me\n"
        );
        assert_eq!(
            db.ingest_artifact_tree_under_write_lock(&destination)
                .unwrap()
                .0,
            tree_id
        );
    }

    #[test]
    fn artifact_materialization_cache_is_tree_keyed_verified_and_copy_safe() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::create_dir_all(source.path().join("pkg")).unwrap();
        fs::write(source.path().join("pkg/index.js"), b"shared-cache\n").unwrap();
        let (tree_id, _) = db.ingest_artifact_tree(source.path()).unwrap();

        let first = db.ensure_artifact_tree_materialization(&tree_id).unwrap();
        assert!(!first.reused);
        assert_eq!(first.tree_root_id, tree_id);
        assert_eq!(
            fs::read(first.storage_path.join("pkg/index.js")).unwrap(),
            b"shared-cache\n"
        );
        let second = db.ensure_artifact_tree_materialization(&tree_id).unwrap();
        assert!(second.reused);
        assert_eq!(first.storage_path, second.storage_path);
        assert_eq!(first.materialization_id, second.materialization_id);
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM artifact_materializations",
                    [],
                    |row| { row.get::<_, i64>(0) }
                )
                .unwrap(),
            1
        );

        let private = workspace.path().join("private-copy");
        super::super::workspace_layer::copy_layer_tree(&first.storage_path, &private).unwrap();
        super::super::workspace_layer::make_tree_writable(&private);
        fs::write(private.join("pkg/index.js"), b"private\n").unwrap();
        assert_eq!(
            fs::read(first.storage_path.join("pkg/index.js")).unwrap(),
            b"shared-cache\n"
        );

        super::super::workspace_layer::make_tree_writable(&first.storage_path);
        fs::write(first.storage_path.join("pkg/index.js"), b"corrupt\n").unwrap();
        let repaired = db.ensure_artifact_tree_materialization(&tree_id).unwrap();
        assert!(!repaired.reused);
        assert_eq!(repaired.storage_path, first.storage_path);
        assert_eq!(
            fs::read(repaired.storage_path.join("pkg/index.js")).unwrap(),
            b"shared-cache\n"
        );

        super::super::workspace_layer::make_tree_writable(&repaired.storage_path);
        fs::remove_dir_all(&repaired.storage_path).unwrap();
        let rebuilt = db.ensure_artifact_tree_materialization(&tree_id).unwrap();
        assert!(!rebuilt.reused);
        assert_eq!(rebuilt.storage_path, repaired.storage_path);
        assert_eq!(rebuilt.logical_bytes, b"shared-cache\n".len() as u64);
        assert_eq!(rebuilt.entry_count, 2);
        assert!(rebuilt.physical_bytes > 0);
        assert!(rebuilt
            .backend_compatibility
            .starts_with("trail-real-directory/v1/"));

        db.conn
            .execute(
                "DELETE FROM artifact_materializations WHERE materialization_id=?1",
                params![&rebuilt.materialization_id],
            )
            .unwrap();
        super::super::workspace_layer::make_tree_writable(&rebuilt.storage_path);
        let adopted = db.ensure_artifact_tree_materialization(&tree_id).unwrap();
        assert!(adopted.reused);
        assert_eq!(adopted.storage_path, rebuilt.storage_path);
        assert!(fs::metadata(&adopted.storage_path)
            .unwrap()
            .permissions()
            .readonly());

        db.conn
            .execute(
                "UPDATE artifact_materializations SET state='building'
                 WHERE materialization_id=?1",
                params![&adopted.materialization_id],
            )
            .unwrap();
        let recovered = db.ensure_artifact_tree_materialization(&tree_id).unwrap();
        assert!(!recovered.reused);
        assert_eq!(
            fs::read(recovered.storage_path.join("pkg/index.js")).unwrap(),
            b"shared-cache\n"
        );
    }

    #[test]
    fn artifact_materialization_cache_eviction_releases_object_gc_lease() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("artifact"), b"rebuildable cache\n").unwrap();
        let (tree_id, _) = db.ingest_artifact_tree(source.path()).unwrap();
        let materialization = db.ensure_artifact_tree_materialization(&tree_id).unwrap();

        assert_eq!(db.gc(false).unwrap().pruned_objects, 0);
        let preview = db.workspace_cache_gc(true, Some(0)).unwrap();
        assert!(preview.candidates.iter().any(|candidate| {
            candidate.kind == "artifact_materialization"
                && candidate.id == materialization.materialization_id
        }));
        assert_eq!(
            preview.artifact_storage.materialized_bytes,
            materialization.physical_bytes
        );
        assert!(preview.artifact_storage.reclaimable_bytes >= materialization.physical_bytes);
        assert_eq!(
            preview
                .artifact_storage
                .materialized_bytes
                .saturating_add(preview.artifact_storage.demand_loaded_bytes)
                .saturating_add(preview.artifact_storage.unknown_bytes),
            preview.cache_physical_bytes_before
        );
        assert!(materialization.storage_path.exists());

        let collected = db.workspace_cache_gc(false, Some(0)).unwrap();
        assert!(collected.deleted.iter().any(|candidate| {
            candidate.kind == "artifact_materialization"
                && candidate.id == materialization.materialization_id
        }));
        assert!(!materialization.storage_path.exists());
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM artifact_materializations",
                    [],
                    |row| { row.get::<_, u64>(0) }
                )
                .unwrap(),
            0
        );
        let after_eviction = db.workspace_cache_gc(true, Some(0)).unwrap();
        assert_eq!(after_eviction.artifact_storage.materialized_bytes, 0);

        let object_gc = db.gc(false).unwrap();
        assert!(object_gc.pruned_objects > 0);
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM artifact_objects", [], |row| {
                    row.get::<_, u64>(0)
                })
                .unwrap(),
            0
        );
    }

    #[test]
    fn backup_restore_preserves_artifact_authority_and_private_source_state() {
        let workspace = tempfile::tempdir().unwrap();
        fs::write(
            workspace.path().join("Cargo.toml"),
            "[package]\nname='backup-artifact'\nversion='0.1.0'\n",
        )
        .unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(workspace.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "backup-artifact",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
        let lane = db.lane_details("backup-artifact").unwrap().branch;
        let mountpoint = db.default_lane_workdir_path("backup-artifact").unwrap();
        let view = db
            .create_workspace_view(
                &lane.lane_id,
                &lane.head_change,
                &lane.head_root,
                "test-cow",
                &mountpoint,
            )
            .unwrap();
        let source_upper = PathBuf::from(&view.source_upper);
        let mut journal = super::workdir::ViewMutationJournal::open(&source_upper).unwrap();
        journal
            .append(
                super::workdir::ViewMutationKind::Create,
                "agent-change.rs",
                None,
            )
            .unwrap();
        fs::write(source_upper.join("agent-change.rs"), "private source\n").unwrap();

        let source_root = db.resolve_refish("main").unwrap().root_id;
        let (snapshot_id, _) = db
            .put_artifact_resolution_snapshot(
                fixture_plan(source_root.clone()),
                b"version = 4\n".to_vec(),
                BTreeMap::new(),
                BTreeMap::new(),
                Vec::new(),
                false,
            )
            .unwrap();
        let artifact_source = tempfile::tempdir().unwrap();
        fs::create_dir_all(artifact_source.path().join("deps")).unwrap();
        fs::write(
            artifact_source.path().join("deps/library.rlib"),
            "immutable artifact\n",
        )
        .unwrap();
        let (tree_id, _) = db.ingest_artifact_tree(artifact_source.path()).unwrap();
        let mut desired_material = fixture_desired_material(source_root.clone());
        desired_material.resolution_snapshot_id = Some(snapshot_id.clone());
        let desired_key = artifact_desired_key_v2(desired_material).unwrap();
        let (envelope_id, quarantined) = db
            .put_artifact_envelope_under_write_lock(ArtifactEnvelopeV1 {
                version: ARTIFACT_ENVELOPE_VERSION,
                desired_identity: ArtifactDesiredIdentityV1::ArtifactDesiredV2 {
                    desired_key: desired_key.clone(),
                },
                tree_root_id: tree_id.clone(),
                component_id: "cargo:root".into(),
                output_name: "target".into(),
                output_policy: EnvironmentOutputPolicy::ImmutableSeedPrivate,
                portability_scope: "workspace".into(),
                trust_scope: "builtin".into(),
                secret_taint: ArtifactSecretTaintV1::Clear,
                resolution_snapshot_id: Some(snapshot_id.clone()),
                validation_receipt_ids: Vec::new(),
            })
            .unwrap();
        assert!(!quarantined);
        db.verify_ready_artifact_envelope_under_write_lock(&envelope_id, &tree_id)
            .unwrap();
        let materialization = db.ensure_artifact_tree_materialization(&tree_id).unwrap();

        let generation_id = "envgen_backup_artifact";
        db.conn
            .execute(
                "INSERT INTO environment_generations(
                     generation_id,view_id,generation_sequence,source_root,specification_digest,
                     predecessor_generation_id,state,created_at,activated_at,retired_at)
                 VALUES(?1,?2,1,?3,'backup-spec',NULL,'active',1,1,NULL)",
                params![generation_id, &view.view_id, source_root.0],
            )
            .unwrap();
        db.conn
            .execute(
                "INSERT INTO environment_view_generations(view_id,generation_id,updated_at)
                 VALUES(?1,?2,1)",
                params![&view.view_id, generation_id],
            )
            .unwrap();
        let binding_identity = format!(
            "artifact_binding_{}",
            crate::ids::short_hash(
                format!("{generation_id}\0cargo:root\0target\0{envelope_id}").as_bytes(),
                32,
            )
        );
        db.conn
            .execute(
                "INSERT INTO artifact_generation_bindings(
                     binding_id,generation_id,component_id,output_name,desired_key,envelope_id,
                     tree_root_id,binding_identity,created_at)
                 VALUES(?1,?2,'cargo:root','target',?3,?4,?5,?6,1)",
                params![
                    format!(
                        "binding_{}",
                        crate::ids::short_hash(binding_identity.as_bytes(), 32)
                    ),
                    generation_id,
                    desired_key.0,
                    envelope_id.0,
                    tree_id.0,
                    binding_identity,
                ],
            )
            .unwrap();
        let cache_path = workspace.path().join(".trail/cache/namespaces/backup");
        fs::create_dir_all(&cache_path).unwrap();
        db.conn
            .execute(
                "INSERT INTO environment_cache_namespaces(
                     namespace_id,adapter_identity,cache_name,protocol,access,authority,scope,
                     compatibility_json,storage_path,last_used_at,created_at)
                 VALUES('cache_backup','trail/test@1','registry','content-v1','read_write',
                        'performance_only','workspace',X'7B7D',?1,1,1)",
                params![cache_path.to_string_lossy()],
            )
            .unwrap();

        let backup_parent = tempfile::tempdir().unwrap();
        let backup = backup_parent.path().join("portable-backup");
        let created = db.create_backup(&backup, false).unwrap();
        assert_eq!(created.retained_private_views, 1);
        assert!(created.retained_private_bytes > 0);
        assert!(created.rebuildable_materializations >= 1);
        assert!(created.rebuildable_materialization_bytes >= materialization.physical_bytes);
        assert_eq!(created.rebuildable_performance_caches, 1);

        let backup_conn = Connection::open(backup.join(DB_RELATIVE_PATH)).unwrap();
        for table in [
            "artifact_resolution_snapshots",
            "artifact_envelopes",
            "artifact_attestations",
            "artifact_generation_bindings",
            "environment_generations",
        ] {
            let count: i64 = backup_conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 1, "backup lost authoritative table {table}");
        }
        assert_eq!(
            backup_conn
                .query_row(
                    "SELECT COUNT(*) FROM artifact_materializations",
                    [],
                    |row| { row.get::<_, i64>(0) }
                )
                .unwrap(),
            0
        );
        assert_eq!(
            backup_conn
                .query_row(
                    "SELECT COUNT(*) FROM environment_cache_namespaces",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
        let backed_up_private = backup
            .join("views")
            .join(&view.view_id)
            .join("source-upper/agent-change.rs");
        assert_eq!(
            fs::read_to_string(&backed_up_private).unwrap(),
            "private source\n"
        );
        let verified = Trail::verify_backup(&backup).unwrap();
        assert!(verified.valid, "{:?}", verified.errors);
        assert_eq!(verified.retained_private_views, 1);
        fs::write(&backed_up_private, "tampered source\n").unwrap();
        let tampered = Trail::verify_backup(&backup).unwrap();
        assert!(!tampered.valid);
        assert!(tampered
            .errors
            .iter()
            .any(|error| error.contains("retained private SHA-256 mismatch")));
        fs::write(&backed_up_private, "private source\n").unwrap();
        assert!(Trail::verify_backup(&backup).unwrap().valid);

        drop(backup_conn);
        drop(db);
        let restored = tempfile::tempdir().unwrap();
        let restore = Trail::restore_backup(restored.path(), &backup, false).unwrap();
        assert_eq!(restore.restored_private_views, 1);
        assert_eq!(
            restore.rebuildable_materializations,
            created.rebuildable_materializations
        );
        let restored_db = Trail::open(restored.path()).unwrap();
        let restored_view = restored_db
            .lane_workspace_view("backup-artifact")
            .unwrap()
            .unwrap();
        assert_eq!(
            fs::read_to_string(Path::new(&restored_view.source_upper).join("agent-change.rs"))
                .unwrap(),
            "private source\n"
        );
        assert!(Path::new(&restored_view.generated_upper).is_dir());
        assert_eq!(
            restored_db
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM artifact_generation_bindings",
                    [],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            1
        );
        assert_eq!(
            restored_db
                .conn
                .query_row(
                    "SELECT state FROM environment_generations WHERE generation_id=?1",
                    params![generation_id],
                    |row| row.get::<_, String>(0)
                )
                .unwrap(),
            "retired"
        );
        assert_eq!(
            restored_db
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM artifact_materializations",
                    [],
                    |row| { row.get::<_, i64>(0) }
                )
                .unwrap(),
            0
        );
        let rebuilt = restored_db
            .ensure_artifact_tree_materialization(&tree_id)
            .unwrap();
        assert!(!rebuilt.reused);
        assert_eq!(
            fs::read_to_string(rebuilt.storage_path.join("deps/library.rlib")).unwrap(),
            "immutable artifact\n"
        );
    }

    #[test]
    fn artifact_materialization_rejects_noncanonical_database_path_without_touching_it() {
        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("artifact"), b"safe\n").unwrap();
        let (tree_id, _) = db.ingest_artifact_tree(source.path()).unwrap();
        let materialization = db.ensure_artifact_tree_materialization(&tree_id).unwrap();
        let external = tempfile::tempdir().unwrap();
        let sentinel = external.path().join("sentinel");
        fs::write(&sentinel, b"keep me\n").unwrap();
        db.conn
            .execute(
                "UPDATE artifact_materializations SET storage_path=?1
                 WHERE materialization_id=?2",
                params![
                    external.path().to_string_lossy(),
                    &materialization.materialization_id
                ],
            )
            .unwrap();

        let error = db
            .ensure_artifact_tree_materialization(&tree_id)
            .unwrap_err();
        assert!(matches!(error, Error::Corrupt(_)));
        assert_eq!(fs::read(&sentinel).unwrap(), b"keep me\n");
        assert!(external.path().is_dir());
    }

    #[cfg(unix)]
    #[test]
    fn artifact_materialization_rejects_external_cache_and_root_symlinks() {
        use std::os::unix::fs::symlink;

        let workspace = tempfile::tempdir().unwrap();
        Trail::init(workspace.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(workspace.path()).unwrap();
        let source = tempfile::tempdir().unwrap();
        fs::write(source.path().join("artifact"), b"safe\n").unwrap();
        let (tree_id, _) = db.ingest_artifact_tree(source.path()).unwrap();
        let external_parent = tempfile::tempdir().unwrap();
        let parent_sentinel = external_parent.path().join("sentinel");
        fs::write(&parent_sentinel, b"parent safe\n").unwrap();
        let staging = db.workspace_environment_staging_parent().unwrap();
        let materializations = staging.parent().unwrap().join("artifact-materializations");
        symlink(external_parent.path(), &materializations).unwrap();

        let error = db
            .ensure_artifact_tree_materialization(&tree_id)
            .unwrap_err();
        assert!(matches!(error, Error::InvalidPath { .. }));
        assert_eq!(fs::read(&parent_sentinel).unwrap(), b"parent safe\n");
        assert_eq!(fs::read_dir(external_parent.path()).unwrap().count(), 1);

        fs::remove_file(&materializations).unwrap();
        let materialization = db.ensure_artifact_tree_materialization(&tree_id).unwrap();
        super::super::workspace_layer::make_tree_writable(&materialization.storage_path);
        fs::remove_dir_all(&materialization.storage_path).unwrap();
        let external_root = tempfile::tempdir().unwrap();
        let root_sentinel = external_root.path().join("sentinel");
        fs::write(&root_sentinel, b"root safe\n").unwrap();
        symlink(external_root.path(), &materialization.storage_path).unwrap();

        let error = db
            .ensure_artifact_tree_materialization(&tree_id)
            .unwrap_err();
        assert!(matches!(error, Error::InvalidPath { .. }));
        assert_eq!(fs::read(&root_sentinel).unwrap(), b"root safe\n");
        assert!(!fs::metadata(&root_sentinel)
            .unwrap()
            .permissions()
            .readonly());
    }

    fn artifact_file_chunk_ids(db: &Trail, file_id: &ArtifactFileId) -> BTreeSet<ArtifactChunkId> {
        let file: ArtifactFileNodeV1 = db
            .get_object(ARTIFACT_FILE_NODE_KIND, &artifact_object_id(db, &file_id.0))
            .unwrap();
        let ArtifactFileContentV1::Chunks { chunk_list_id } = file.content else {
            panic!("large fixture must use chunks");
        };
        let list: ArtifactChunkListV1 = db
            .get_object(
                ARTIFACT_CHUNK_LIST_KIND,
                &artifact_object_id(db, &chunk_list_id.0),
            )
            .unwrap();
        list.chunks
            .into_iter()
            .map(|chunk| chunk.chunk_id)
            .collect()
    }

    fn artifact_object_id(db: &Trail, artifact_id: &str) -> ObjectId {
        ObjectId(
            db.conn
                .query_row(
                    "SELECT object_id FROM artifact_objects WHERE artifact_id=?1",
                    params![artifact_id],
                    |row| row.get::<_, String>(0),
                )
                .unwrap(),
        )
    }

    #[test]
    fn resolution_plan_is_canonical_and_bounded() {
        let mut plan = fixture_plan(ObjectId("object_source".into()));
        plan.allowed_authorities = vec![
            "index.crates.io:443".into(),
            "crates.io:443".into(),
            "index.crates.io:443".into(),
        ];
        normalize_artifact_resolution_plan(&mut plan).unwrap();
        assert_eq!(
            plan.allowed_authorities,
            vec!["crates.io:443", "index.crates.io:443"]
        );

        plan.argv = vec!["cargo".into(); MAX_RESOLUTION_ARGV + 1];
        assert!(normalize_artifact_resolution_plan(&mut plan).is_err());
    }

    #[test]
    fn resolution_snapshot_is_content_addressed_reused_and_refreshed() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let source_root = db.resolve_refish("main").unwrap().root_id;
        let plan = fixture_plan(source_root);

        let (first_id, first) = db
            .put_artifact_resolution_snapshot(
                plan.clone(),
                b"version = 4\n".to_vec(),
                BTreeMap::from([("package:a".into(), "1.0.0".into())]),
                BTreeMap::from([("package:a".into(), "22".repeat(32))]),
                vec!["index.crates.io:443".into()],
                false,
            )
            .unwrap();
        let (reused_id, reused) = db
            .put_artifact_resolution_snapshot(
                plan.clone(),
                b"version = 4\n".to_vec(),
                BTreeMap::new(),
                BTreeMap::new(),
                vec![],
                false,
            )
            .unwrap();
        assert_eq!(reused_id, first_id);
        assert_eq!(reused, first);
        assert_eq!(
            db.artifact_resolution_snapshot_content(&first).unwrap(),
            b"version = 4\n"
        );

        let (next_id, next) = db
            .put_artifact_resolution_snapshot(
                plan,
                b"version = 4\n# refreshed\n".to_vec(),
                BTreeMap::new(),
                BTreeMap::new(),
                vec![],
                true,
            )
            .unwrap();
        assert_ne!(next_id, first_id);
        assert_eq!(next.predecessor_snapshot_id, Some(first_id));
        assert_eq!(
            db.artifact_resolution_snapshot_for_proposal("proposal_fixture")
                .unwrap()
                .unwrap()
                .0,
            next_id
        );
    }

    #[test]
    fn fsck_reads_raw_resolution_snapshot_bytes_past_the_object_cache() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let source_root = db.resolve_refish("main").unwrap().root_id;
        let (_, snapshot) = db
            .put_artifact_resolution_snapshot(
                fixture_plan(source_root),
                b"version = 4\n".to_vec(),
                BTreeMap::new(),
                BTreeMap::new(),
                vec![],
                false,
            )
            .unwrap();
        assert_eq!(
            db.artifact_resolution_snapshot_content(&snapshot).unwrap(),
            b"version = 4\n"
        );

        db.conn
            .execute(
                "UPDATE objects SET bytes=X'00' WHERE object_id=?1",
                params![snapshot.content_object_id.0],
            )
            .unwrap();

        let errors = db.validate_artifact_cas_integrity().unwrap();
        assert!(errors.iter().any(|error| {
            error.contains("artifact resolution snapshot")
                && error.contains("explicit component resolution with refresh")
        }));
    }

    #[test]
    fn resolution_snapshot_rejects_undeclared_authority_and_implicit_drift() {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let source_root = db.resolve_refish("main").unwrap().root_id;
        let plan = fixture_plan(source_root);
        let error = db
            .put_artifact_resolution_snapshot(
                plan.clone(),
                b"lock".to_vec(),
                BTreeMap::new(),
                BTreeMap::new(),
                vec!["evil.example:443".into()],
                false,
            )
            .unwrap_err();
        assert!(error.to_string().contains("undeclared"));

        db.put_artifact_resolution_snapshot(
            plan.clone(),
            b"lock".to_vec(),
            BTreeMap::new(),
            BTreeMap::new(),
            vec![],
            false,
        )
        .unwrap();
        let error = db
            .put_artifact_resolution_snapshot(
                plan,
                b"different lock".to_vec(),
                BTreeMap::new(),
                BTreeMap::new(),
                vec![],
                false,
            )
            .unwrap_err();
        assert!(error.to_string().contains("explicit refresh"));
    }

    #[test]
    fn resolve_component_reuses_until_deliberate_refresh() {
        let (_temp, db, source_root) = initialized_resolution_fixture();
        let plan = executable_fixture_plan(&db, source_root);
        let first = db
            .resolve_artifact_component(
                ArtifactResolutionRequestV1 {
                    plan: plan.clone(),
                    candidate: fixture_candidate(b"version = 4\n"),
                },
                false,
            )
            .unwrap();
        assert_eq!(first.decision, ArtifactResolutionDecisionV1::Resolved);
        assert!(first.attempt.is_some());

        let reused = db
            .resolve_artifact_component(
                ArtifactResolutionRequestV1 {
                    plan: plan.clone(),
                    candidate: fixture_candidate(b"this candidate must not advance selection"),
                },
                false,
            )
            .unwrap();
        assert_eq!(reused.decision, ArtifactResolutionDecisionV1::Reused);
        assert_eq!(reused.snapshot_id, first.snapshot_id);
        assert!(reused.attempt.is_none());
        assert_eq!(db.artifact_resolution_attempts().unwrap().len(), 1);

        let refreshed = db
            .resolve_artifact_component(
                ArtifactResolutionRequestV1 {
                    plan,
                    candidate: fixture_candidate(b"version = 4\n# deliberate refresh\n"),
                },
                true,
            )
            .unwrap();
        assert_eq!(refreshed.decision, ArtifactResolutionDecisionV1::Refreshed);
        assert_ne!(refreshed.snapshot_id, first.snapshot_id);
        assert_eq!(
            refreshed.snapshot.predecessor_snapshot_id,
            Some(first.snapshot_id)
        );
        assert_eq!(db.artifact_resolution_attempts().unwrap().len(), 2);
    }

    #[test]
    fn resolve_all_is_deterministic_and_rejects_mixed_roots() {
        let (_temp, db, source_root) = initialized_resolution_fixture();
        let first = executable_fixture_plan(&db, source_root.clone());
        let mut second = first.clone();
        second.component_id = "node:root".into();
        second.proposal_key = "proposal_node_fixture".into();
        second.adapter_identity = "trail.builtin/node@1".into();

        let report = db
            .resolve_all_artifact_components(
                vec![
                    ArtifactResolutionRequestV1 {
                        plan: second,
                        candidate: fixture_candidate(b"node-lock"),
                    },
                    ArtifactResolutionRequestV1 {
                        plan: first.clone(),
                        candidate: fixture_candidate(b"cargo-lock"),
                    },
                ],
                false,
            )
            .unwrap();
        assert_eq!(
            report
                .components
                .iter()
                .map(|component| component.component_id.as_str())
                .collect::<Vec<_>>(),
            vec!["cargo:root", "node:root"]
        );

        let mut mixed = first;
        mixed.component_id = "mixed".into();
        mixed.proposal_key = "proposal_mixed".into();
        mixed.source_root = ObjectId("object_other_root".into());
        let error = db
            .resolve_all_artifact_components(
                vec![
                    ArtifactResolutionRequestV1 {
                        plan: executable_fixture_plan(&db, source_root),
                        candidate: fixture_candidate(b"one"),
                    },
                    ArtifactResolutionRequestV1 {
                        plan: mixed,
                        candidate: fixture_candidate(b"two"),
                    },
                ],
                false,
            )
            .unwrap_err();
        assert!(error.to_string().contains("one source root"));
    }

    #[test]
    fn resolve_component_records_malformed_and_output_limit_failures() {
        let (_temp, db, source_root) = initialized_resolution_fixture();
        let plan = executable_fixture_plan(&db, source_root);
        let mut malformed = fixture_candidate(b"");
        malformed.stderr = b"malformed resolver output".to_vec();
        let error = db
            .resolve_artifact_component(
                ArtifactResolutionRequestV1 {
                    plan: plan.clone(),
                    candidate: malformed,
                },
                false,
            )
            .unwrap_err();
        assert!(error.to_string().contains("malformed snapshot"));
        assert!(db
            .artifact_resolution_snapshot_for_proposal(&plan.proposal_key)
            .unwrap()
            .is_none());

        let mut limited_plan = plan;
        limited_plan.proposal_key = "proposal_output_limit".into();
        limited_plan.limits.stdout_bytes = 4;
        let mut oversized = fixture_candidate(b"valid");
        oversized.stdout = b"too much output".to_vec();
        let error = db
            .resolve_artifact_component(
                ArtifactResolutionRequestV1 {
                    plan: limited_plan.clone(),
                    candidate: oversized,
                },
                false,
            )
            .unwrap_err();
        assert!(error.to_string().contains("capture limit"));
        assert!(db
            .artifact_resolution_snapshot_for_proposal(&limited_plan.proposal_key)
            .unwrap()
            .is_none());
        let attempts = db.artifact_resolution_attempts().unwrap();
        assert_eq!(attempts.len(), 2);
        assert!(attempts.iter().all(|attempt| {
            attempt.status == ArtifactResolutionAttemptStatusV1::Failed
                && attempt.failure_receipt_object_id.is_some()
        }));
    }

    #[test]
    fn resolve_component_keeps_secret_tainted_candidate_out_of_shared_cas() {
        let (_temp, db, source_root) = initialized_resolution_fixture();
        let plan = executable_fixture_plan(&db, source_root);
        let secret = b"credential-value-never-store".to_vec();
        let mut candidate = fixture_candidate(b"snapshot credential-value-never-store");
        candidate.stdout = b"stdout credential-value-never-store".to_vec();
        candidate.stderr = b"stderr credential-value-never-store".to_vec();
        candidate.redactions = vec![secret.clone()];

        let error = db
            .resolve_artifact_component(
                ArtifactResolutionRequestV1 {
                    plan: plan.clone(),
                    candidate,
                },
                false,
            )
            .unwrap_err();
        assert!(error.to_string().contains("private, non-promotable"));
        assert!(db
            .artifact_resolution_snapshot_for_proposal(&plan.proposal_key)
            .unwrap()
            .is_none());
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM objects WHERE kind=?1",
                    params![ARTIFACT_RESOLUTION_CONTENT_KIND],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            0
        );

        let attempt = db.artifact_resolution_attempts().unwrap().pop().unwrap();
        assert_eq!(
            attempt.failure_code.as_deref(),
            Some("secret_tainted_output_private_only")
        );
        let receipt: ArtifactResolutionFailureReceiptV1 = db
            .get_object(
                ARTIFACT_RESOLUTION_FAILURE_KIND,
                attempt.failure_receipt_object_id.as_ref().unwrap(),
            )
            .unwrap();
        assert_eq!(
            receipt.secret_taint,
            ArtifactSecretTaintV1::Tainted {
                channels: vec!["resolver_credential".into()]
            }
        );
        for capture_id in [attempt.stdout_object_id, attempt.stderr_object_id]
            .into_iter()
            .flatten()
        {
            let capture: ArtifactResolutionCaptureV1 = db
                .get_object(ARTIFACT_RESOLUTION_CAPTURE_KIND, &capture_id)
                .unwrap();
            assert!(!capture
                .bytes
                .windows(secret.len())
                .any(|bytes| bytes == secret));
        }
        let durable_objects = db
            .conn
            .prepare("SELECT bytes FROM objects ORDER BY object_id")
            .unwrap()
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(durable_objects
            .iter()
            .all(|bytes| !bytes.windows(secret.len()).any(|window| window == secret)));
    }

    #[test]
    fn resolve_component_rejects_stale_source_and_tool_identity() {
        let (_temp, db, source_root) = initialized_resolution_fixture();
        let plan = executable_fixture_plan(&db, source_root);

        let mut stale_source = plan.clone();
        stale_source.readable_inputs[0].content_hash = "00".repeat(32);
        let error = db
            .resolve_artifact_component(
                ArtifactResolutionRequestV1 {
                    plan: stale_source,
                    candidate: fixture_candidate(b"stale"),
                },
                false,
            )
            .unwrap_err();
        assert!(error.to_string().contains("changed after planning"));

        let mut stale_tool = plan;
        stale_tool.executable_identity = "sha256:stale-tool".into();
        let error = db
            .resolve_artifact_component(
                ArtifactResolutionRequestV1 {
                    plan: stale_tool,
                    candidate: fixture_candidate(b"stale"),
                },
                false,
            )
            .unwrap_err();
        assert!(error.to_string().contains("executable"));
        assert!(error.to_string().contains("changed after planning"));
    }

    #[test]
    fn resolution_attempt_is_fenced_cancelled_and_redacts_credentials() {
        let (_temp, db, source_root) = initialized_resolution_fixture();
        let mut plan = executable_fixture_plan(&db, source_root);
        plan.credential_handles = vec!["registry_credentials".into()];
        let (fence, started) = db.begin_artifact_resolution_attempt(plan).unwrap();
        assert_eq!(started.status, ArtifactResolutionAttemptStatusV1::Running);
        assert!(db.heartbeat_artifact_resolution_attempt(&fence).unwrap());

        let cancelling = db
            .cancel_artifact_resolution_attempt(&fence.attempt_id)
            .unwrap();
        assert!(cancelling.cancel_requested);
        assert!(!db.heartbeat_artifact_resolution_attempt(&fence).unwrap());
        let secret = b"credential-value-never-store".to_vec();
        let finished = db
            .finish_artifact_resolution_attempt_failure(
                &fence,
                ArtifactResolutionAttemptFailure {
                    code: "cancelled_by_user",
                    message: "credential-value-never-store was cancelled",
                    contacted_authorities: vec!["index.crates.io:443".into()],
                    stdout: b"stdout credential-value-never-store",
                    stderr: b"stderr credential-value-never-store",
                    stdout_original_bytes: None,
                    stderr_original_bytes: None,
                    redactions: std::slice::from_ref(&secret),
                    cancelled: true,
                },
            )
            .unwrap();
        assert_eq!(
            finished.status,
            ArtifactResolutionAttemptStatusV1::Cancelled
        );
        assert!(finished
            .failure_message
            .as_ref()
            .unwrap()
            .contains("[REDACTED]"));
        let stdout: ArtifactResolutionCaptureV1 = db
            .get_object(
                ARTIFACT_RESOLUTION_CAPTURE_KIND,
                finished.stdout_object_id.as_ref().unwrap(),
            )
            .unwrap();
        assert!(!stdout
            .bytes
            .windows(secret.len())
            .any(|bytes| bytes == secret));
        let receipt: ArtifactResolutionFailureReceiptV1 = db
            .get_object(
                ARTIFACT_RESOLUTION_FAILURE_KIND,
                finished.failure_receipt_object_id.as_ref().unwrap(),
            )
            .unwrap();
        assert!(receipt.message.contains("[REDACTED]"));
        assert_eq!(
            receipt.authority_evidence.credential_handles,
            vec!["registry_credentials"]
        );
        assert!(receipt.authority_evidence.credential_values_redacted);
        assert_eq!(
            receipt.secret_taint,
            ArtifactSecretTaintV1::Tainted {
                channels: vec!["resolver_credential".into()]
            }
        );
        assert!(!serde_json::to_vec(&finished)
            .unwrap()
            .windows(secret.len())
            .any(|bytes| bytes == secret));
    }

    #[test]
    fn resolution_attempt_rejects_stale_fence_and_bounds_capture() {
        let (_temp, db, source_root) = initialized_resolution_fixture();
        let mut plan = executable_fixture_plan(&db, source_root);
        plan.limits.stdout_bytes = 8;
        let (fence, _) = db.begin_artifact_resolution_attempt(plan).unwrap();
        let mut stale = fence.clone();
        stale.owner_generation += 1;
        assert!(!db.heartbeat_artifact_resolution_attempt(&stale).unwrap());
        assert!(db
            .finish_artifact_resolution_attempt_failure(
                &stale,
                ArtifactResolutionAttemptFailure {
                    code: "failed",
                    message: "failure",
                    contacted_authorities: vec![],
                    stdout: b"output",
                    stderr: b"",
                    stdout_original_bytes: None,
                    stderr_original_bytes: None,
                    redactions: &[],
                    cancelled: false,
                },
            )
            .unwrap_err()
            .to_string()
            .contains("exact owner fence"));

        let finished = db
            .finish_artifact_resolution_attempt_failure(
                &fence,
                ArtifactResolutionAttemptFailure {
                    code: "resolver_failed",
                    message: "resolver failed",
                    contacted_authorities: vec![],
                    stdout: b"0123456789abcdef",
                    stderr: b"",
                    stdout_original_bytes: None,
                    stderr_original_bytes: None,
                    redactions: &[],
                    cancelled: false,
                },
            )
            .unwrap();
        let capture: ArtifactResolutionCaptureV1 = db
            .get_object(
                ARTIFACT_RESOLUTION_CAPTURE_KIND,
                finished.stdout_object_id.as_ref().unwrap(),
            )
            .unwrap();
        assert_eq!(capture.bytes, b"01234567");
        assert_eq!(capture.original_bytes, 16);
        assert!(capture.truncated);
    }

    #[test]
    fn resolution_attempt_singleflight_and_open_recovery_are_durable() {
        let (temp, db, source_root) = initialized_resolution_fixture();
        let plan = executable_fixture_plan(&db, source_root);
        let (fence, _) = db.begin_artifact_resolution_attempt(plan.clone()).unwrap();
        let error = db.begin_artifact_resolution_attempt(plan).unwrap_err();
        assert!(error.to_string().contains("already resolving"));
        db.conn
            .execute(
                "UPDATE artifact_resolution_attempts
                 SET owner_pid=?1, owner_start_token='dead-owner'
                 WHERE attempt_id=?2",
                params![i64::from(u32::MAX), fence.attempt_id.0],
            )
            .unwrap();
        drop(db);

        let reopened = Trail::open(temp.path()).unwrap();
        let recovered = reopened
            .artifact_resolution_attempt(&fence.attempt_id)
            .unwrap();
        assert_eq!(
            recovered.status,
            ArtifactResolutionAttemptStatusV1::Abandoned
        );
        assert_eq!(
            recovered.failure_code.as_deref(),
            Some("resolver_owner_lost")
        );
        assert!(recovered.failure_receipt_object_id.is_some());

        let plan = executable_fixture_plan(&reopened, recovered.source_root.clone());
        let (_, successor) = reopened.begin_artifact_resolution_attempt(plan).unwrap();
        assert_eq!(successor.owner_generation, recovered.owner_generation + 1);
    }

    #[test]
    fn resolution_attempt_success_requires_matching_snapshot_and_authority() {
        let (_temp, db, source_root) = initialized_resolution_fixture();
        let plan = executable_fixture_plan(&db, source_root);
        let (fence, _) = db.begin_artifact_resolution_attempt(plan.clone()).unwrap();
        let (snapshot_id, _) = db
            .put_artifact_resolution_snapshot(
                plan,
                b"version = 4\n".to_vec(),
                BTreeMap::new(),
                BTreeMap::new(),
                vec!["index.crates.io:443".into()],
                false,
            )
            .unwrap();
        let error = db
            .finish_artifact_resolution_attempt_success(
                &fence,
                &snapshot_id,
                vec!["undeclared.example:443".into()],
                b"",
                b"",
                &[],
            )
            .unwrap_err();
        assert!(error.to_string().contains("undeclared"));
        let finished = db
            .finish_artifact_resolution_attempt_success(
                &fence,
                &snapshot_id,
                vec!["index.crates.io:443".into()],
                b"resolved",
                b"",
                &[],
            )
            .unwrap();
        assert_eq!(
            finished.status,
            ArtifactResolutionAttemptStatusV1::Succeeded
        );
        assert_eq!(finished.snapshot_id, Some(snapshot_id));
    }

    #[test]
    fn object_gc_collects_unbound_artifact_envelopes_and_content_graphs() {
        let (workspace, mut db, source_root) = initialized_resolution_fixture();
        let candidate = tempfile::tempdir().unwrap();
        for index in 0..130 {
            fs::write(
                candidate.path().join(format!("result-{index:03}.bin")),
                format!("unbound artifact {index}\n"),
            )
            .unwrap();
        }
        let (tree_id, _) = db.ingest_artifact_tree(candidate.path()).unwrap();
        let desired_key = artifact_desired_key_v2(fixture_desired_material(source_root)).unwrap();
        let (envelope_id, quarantined) = db
            .put_artifact_envelope_under_write_lock(ArtifactEnvelopeV1 {
                version: ARTIFACT_ENVELOPE_VERSION,
                desired_identity: ArtifactDesiredIdentityV1::ArtifactDesiredV2 { desired_key },
                tree_root_id: tree_id,
                component_id: "cargo:root".into(),
                output_name: "target".into(),
                output_policy: EnvironmentOutputPolicy::ImmutableSeedPrivate,
                portability_scope: "workspace".into(),
                trust_scope: "builtin".into(),
                secret_taint: ArtifactSecretTaintV1::Clear,
                resolution_snapshot_id: None,
                validation_receipt_ids: Vec::new(),
            })
            .unwrap();
        assert!(!quarantined);
        let artifact_object_count = db
            .conn
            .query_row("SELECT COUNT(*) FROM artifact_objects", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap();
        assert!(artifact_object_count > 256);

        let preview = db.gc(true).unwrap();
        assert!(preview.prunable_objects >= artifact_object_count);
        assert_eq!(preview.pruned_objects, 0);
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM artifact_envelopes WHERE envelope_id=?1",
                    params![envelope_id.0],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            1
        );

        Trail::set_gc_test_failure_after_committed_batches_for_current_thread(Some(1));
        let interrupted = db.gc(false).unwrap_err();
        assert!(interrupted
            .to_string()
            .contains("injected object-GC interruption"));
        let remaining_after_interruption = db
            .conn
            .query_row("SELECT COUNT(*) FROM artifact_objects", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap();
        assert!(remaining_after_interruption > 0);
        assert!(remaining_after_interruption < artifact_object_count);

        drop(db);
        let mut db = Trail::open(workspace.path()).unwrap();
        let collected = db.gc(false).unwrap();
        assert!(collected.pruned_objects > 0);
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM artifact_objects", [], |row| {
                    row.get::<_, u64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM artifact_envelopes", [], |row| {
                    row.get::<_, u64>(0)
                })
                .unwrap(),
            0
        );
        assert_eq!(db.gc(false).unwrap().pruned_objects, 0);
        drop(workspace);
    }

    #[test]
    fn object_gc_preserves_shared_chunks_until_the_last_hold_is_removed() {
        let (_workspace, mut db, source_root) = initialized_resolution_fixture();
        let first_candidate = tempfile::tempdir().unwrap();
        let second_candidate = tempfile::tempdir().unwrap();
        let mut shared = vec![0_u8; ARTIFACT_WHOLE_BLOB_MAX_BYTES + 512 * 1024];
        for (index, byte) in shared.iter_mut().enumerate() {
            *byte = ((index.wrapping_mul(31)) % 251) as u8;
        }
        for candidate in [&first_candidate, &second_candidate] {
            fs::write(candidate.path().join("shared.bin"), &shared).unwrap();
        }
        fs::write(first_candidate.path().join("only-first"), b"first\n").unwrap();
        fs::write(second_candidate.path().join("only-second"), b"second\n").unwrap();
        let (first_tree, _) = db.ingest_artifact_tree(first_candidate.path()).unwrap();
        let (second_tree, _) = db.ingest_artifact_tree(second_candidate.path()).unwrap();

        let first_key =
            artifact_desired_key_v2(fixture_desired_material(source_root.clone())).unwrap();
        let mut second_material = fixture_desired_material(source_root);
        second_material.target = "release".into();
        let second_key = artifact_desired_key_v2(second_material).unwrap();
        let envelope = |desired_key, tree_root_id| ArtifactEnvelopeV1 {
            version: ARTIFACT_ENVELOPE_VERSION,
            desired_identity: ArtifactDesiredIdentityV1::ArtifactDesiredV2 { desired_key },
            tree_root_id,
            component_id: "cargo:root".into(),
            output_name: "target".into(),
            output_policy: EnvironmentOutputPolicy::ImmutableSeedPrivate,
            portability_scope: "workspace".into(),
            trust_scope: "builtin".into(),
            secret_taint: ArtifactSecretTaintV1::Clear,
            resolution_snapshot_id: None,
            validation_receipt_ids: Vec::new(),
        };
        let (first_envelope, _) = db
            .put_artifact_envelope_under_write_lock(envelope(first_key, first_tree.clone()))
            .unwrap();
        let (second_envelope, _) = db
            .put_artifact_envelope_under_write_lock(envelope(second_key, second_tree))
            .unwrap();
        let accounting = db.artifact_storage_accounting(None, 13, 7, 5, 11).unwrap();
        let authoritative_bytes = db
            .conn
            .query_row(
                "SELECT COALESCE(SUM(LENGTH(o.bytes)),0)
                 FROM objects o
                 WHERE o.object_id IN (
                     SELECT object_id FROM artifact_objects
                     UNION SELECT object_id FROM artifact_attestations
                 )",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap();
        assert_eq!(
            accounting
                .unique_authoritative_bytes
                .saturating_add(accounting.cross_artifact_shared_bytes),
            authoritative_bytes
        );
        assert!(accounting.unique_authoritative_bytes > 0);
        assert!(accounting.cross_artifact_shared_bytes > 0);
        assert_eq!(
            accounting.logical_bytes,
            (shared.len() * 2 + b"first\n".len() + b"second\n".len()) as u64
        );
        assert_eq!(accounting.lane_private_bytes, 13);
        assert_eq!(accounting.demand_loaded_bytes, 7);
        assert_eq!(accounting.reclaimable_bytes, 5);
        assert_eq!(accounting.unknown_bytes, 11);
        assert_eq!(accounting.prefetched_bytes, 0);

        let materialization = db
            .ensure_artifact_tree_materialization(&first_tree)
            .unwrap();
        let with_materialization = db.artifact_storage_accounting(None, 0, 0, 0, 0).unwrap();
        assert_eq!(
            with_materialization.materialized_bytes,
            materialization.physical_bytes
        );
        db.conn
            .execute(
                "DELETE FROM artifact_materializations WHERE materialization_id=?1",
                params![materialization.materialization_id],
            )
            .unwrap();
        super::super::workspace_layer::make_tree_writable(&materialization.storage_path);
        fs::remove_dir_all(materialization.storage_path).unwrap();
        for (hold_id, envelope_id) in [
            ("hold_first", &first_envelope),
            ("hold_second", &second_envelope),
        ] {
            db.conn
                .execute(
                    "INSERT INTO artifact_holds(
                        hold_id,target_kind,target_id,reason,created_at
                     ) VALUES(?1,'artifact_envelope',?2,'gc-test',?3)",
                    params![hold_id, envelope_id.0, now_ts()],
                )
                .unwrap();
        }
        let chunk_objects = {
            let mut statement = db
                .conn
                .prepare(
                    "SELECT object_id FROM artifact_objects
                     WHERE kind=?1 ORDER BY object_id",
                )
                .unwrap();
            statement
                .query_map(params![ARTIFACT_CHUNK_KIND], |row| row.get::<_, String>(0))
                .unwrap()
                .collect::<std::result::Result<Vec<_>, _>>()
                .unwrap()
        };
        assert!(!chunk_objects.is_empty());

        assert_eq!(db.gc(false).unwrap().pruned_objects, 0);
        db.conn
            .execute("DELETE FROM artifact_holds WHERE hold_id='hold_first'", [])
            .unwrap();
        let first_collection = db.gc(false).unwrap();
        assert!(first_collection.pruned_objects > 0);
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM artifact_envelopes WHERE envelope_id=?1",
                    params![first_envelope.0],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            0
        );
        assert_eq!(
            db.conn
                .query_row(
                    "SELECT COUNT(*) FROM artifact_envelopes WHERE envelope_id=?1",
                    params![second_envelope.0],
                    |row| row.get::<_, u64>(0),
                )
                .unwrap(),
            1
        );
        for object_id in &chunk_objects {
            assert_eq!(
                db.conn
                    .query_row(
                        "SELECT COUNT(*) FROM objects WHERE object_id=?1",
                        params![object_id],
                        |row| row.get::<_, u64>(0),
                    )
                    .unwrap(),
                1
            );
        }

        db.conn
            .execute("DELETE FROM artifact_holds WHERE hold_id='hold_second'", [])
            .unwrap();
        let final_collection = db.gc(false).unwrap();
        assert!(final_collection.pruned_objects > 0);
        assert_eq!(
            db.conn
                .query_row("SELECT COUNT(*) FROM artifact_objects", [], |row| {
                    row.get::<_, u64>(0)
                })
                .unwrap(),
            0
        );
        for object_id in chunk_objects {
            assert_eq!(
                db.conn
                    .query_row(
                        "SELECT COUNT(*) FROM objects WHERE object_id=?1",
                        params![object_id],
                        |row| row.get::<_, u64>(0),
                    )
                    .unwrap(),
                0
            );
        }
    }

    #[test]
    fn artifact_accounting_does_not_multiply_shared_authority_across_1_5_20_lanes() {
        for lane_count in [1_usize, 5, 20] {
            let (_workspace, mut db, source_root) = initialized_resolution_fixture();
            let artifact_source = tempfile::tempdir().unwrap();
            fs::write(
                artifact_source.path().join("shared-output.bin"),
                vec![42_u8; 32 * 1024],
            )
            .unwrap();
            let (tree_id, tree) = db.ingest_artifact_tree(artifact_source.path()).unwrap();
            let desired_key =
                artifact_desired_key_v2(fixture_desired_material(source_root.clone())).unwrap();
            let (envelope_id, quarantined) = db
                .put_artifact_envelope_under_write_lock(ArtifactEnvelopeV1 {
                    version: ARTIFACT_ENVELOPE_VERSION,
                    desired_identity: ArtifactDesiredIdentityV1::ArtifactDesiredV2 {
                        desired_key: desired_key.clone(),
                    },
                    tree_root_id: tree_id.clone(),
                    component_id: "cargo:root".into(),
                    output_name: "target".into(),
                    output_policy: EnvironmentOutputPolicy::ImmutableSeedPrivate,
                    portability_scope: "workspace".into(),
                    trust_scope: "builtin".into(),
                    secret_taint: ArtifactSecretTaintV1::Clear,
                    resolution_snapshot_id: None,
                    validation_receipt_ids: Vec::new(),
                })
                .unwrap();
            assert!(!quarantined);
            let materialization = db.ensure_artifact_tree_materialization(&tree_id).unwrap();

            let mode = if cfg!(target_os = "macos") {
                LaneWorkdirMode::NfsCow
            } else if cfg!(target_os = "windows") {
                LaneWorkdirMode::DokanCow
            } else {
                LaneWorkdirMode::FuseCow
            };
            let mut lane_reports = Vec::new();
            for index in 0..lane_count {
                let lane = format!("accounting-{lane_count}-{index}");
                db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                    &lane,
                    Some("main"),
                    mode.clone(),
                    None,
                    None,
                    None,
                    &[],
                    false,
                )
                .unwrap();
                let view = db.lane_workspace_view(&lane).unwrap().unwrap();
                fs::write(
                    Path::new(&view.generated_upper).join("lane-private.bin"),
                    vec![index as u8; 1024 + index],
                )
                .unwrap();
                let generation_id = format!("envgen_accounting_{lane_count}_{index}");
                db.conn
                    .execute(
                        "INSERT INTO environment_generations(
                             generation_id,view_id,generation_sequence,source_root,
                             specification_digest,predecessor_generation_id,state,created_at,
                             activated_at,retired_at)
                         VALUES(?1,?2,1,?3,'accounting-spec',NULL,'active',1,1,NULL)",
                        params![generation_id, view.view_id, source_root.0],
                    )
                    .unwrap();
                db.conn
                    .execute(
                        "INSERT INTO environment_view_generations(
                             view_id,generation_id,updated_at) VALUES(?1,?2,1)",
                        params![view.view_id, generation_id],
                    )
                    .unwrap();
                db.conn
                    .execute(
                        "INSERT INTO artifact_generation_bindings(
                             binding_id,generation_id,component_id,output_name,desired_key,
                             envelope_id,tree_root_id,binding_identity,created_at)
                         VALUES(?1,?2,'cargo:root','target',?3,?4,?5,?6,1)",
                        params![
                            format!("binding_accounting_{lane_count}_{index}"),
                            generation_id,
                            desired_key.0,
                            envelope_id.0,
                            tree_id.0,
                            format!("identity_accounting_{lane_count}_{index}"),
                        ],
                    )
                    .unwrap();
                lane_reports.push(db.lane_workspace_space(&lane).unwrap());
            }

            let workspace_accounting = db.artifact_storage_accounting(None, 0, 0, 0, 0).unwrap();
            assert_eq!(workspace_accounting.logical_bytes, tree.logical_bytes);
            assert_eq!(workspace_accounting.cross_artifact_shared_bytes, 0);
            assert!(workspace_accounting.unique_authoritative_bytes > 0);
            assert_eq!(
                workspace_accounting.materialized_bytes,
                materialization.physical_bytes
            );
            for report in lane_reports {
                assert_eq!(report.artifact_storage.logical_bytes, tree.logical_bytes);
                assert_eq!(
                    report.artifact_storage.unique_authoritative_bytes,
                    workspace_accounting.unique_authoritative_bytes
                );
                assert_eq!(report.artifact_storage.cross_artifact_shared_bytes, 0);
                assert_eq!(
                    report.artifact_storage.materialized_bytes,
                    materialization.physical_bytes
                );
                assert_eq!(
                    report.artifact_storage.lane_private_bytes,
                    report.lane_exclusive_physical_bytes
                );
                assert!(report.artifact_storage.lane_private_bytes > 0);
                assert_eq!(
                    report.artifact_storage.reclaimable_bytes,
                    materialization.physical_bytes
                );
                assert_eq!(report.artifact_storage.prefetched_bytes, 0);
                assert_eq!(report.artifact_storage.demand_loaded_bytes, 0);
                assert_eq!(report.artifact_storage.unknown_bytes, 0);
            }
        }
    }

    fn synthetic_artifact_tree(
        db: &Trail,
        entry_count: u64,
    ) -> (ArtifactTreeId, ArtifactTreeRootV1) {
        const FILES_PER_DIRECTORY: u64 = 1_000;

        assert!(matches!(entry_count, 10_000 | 100_000 | 1_000_000));
        let directory_count = entry_count.div_ceil(FILES_PER_DIRECTORY + 1);
        let file_count = entry_count - directory_count;
        let required_directories = file_count.div_ceil(FILES_PER_DIRECTORY);
        assert!(required_directories <= directory_count);
        assert!(directory_count - required_directories <= 1);

        let file_bytes = b"shared immutable artifact content\n";
        let file_id = db.ingest_artifact_file_bytes(file_bytes, 0o644).unwrap();
        let mut root_entries = Vec::with_capacity(directory_count as usize);
        let mut remaining_files = file_count;
        for directory_index in 0..directory_count {
            let files_in_directory = remaining_files.min(FILES_PER_DIRECTORY);
            let entries = (0..files_in_directory)
                .map(|file_index| ArtifactDirectoryEntryV1 {
                    name: format!("file-{file_index:04}.bin"),
                    target: ArtifactDirectoryEntryTargetV1::File {
                        node_id: file_id.clone(),
                    },
                })
                .collect();
            let directory = canonical_artifact_directory_node(ArtifactDirectoryNodeV1 {
                version: ARTIFACT_DIRECTORY_NODE_VERSION,
                entries,
            })
            .unwrap();
            let (directory_id, _) = encode_artifact_directory_node(directory.clone()).unwrap();
            db.put_artifact_cas_object(
                &directory_id.0,
                ARTIFACT_DIRECTORY_NODE_KIND,
                ARTIFACT_DIRECTORY_NODE_VERSION,
                files_in_directory * file_bytes.len() as u64,
                &directory,
            )
            .unwrap();
            root_entries.push(ArtifactDirectoryEntryV1 {
                name: format!("bucket-{directory_index:04}"),
                target: ArtifactDirectoryEntryTargetV1::Directory {
                    node_id: directory_id,
                },
            });
            remaining_files -= files_in_directory;
        }
        assert_eq!(remaining_files, 0);

        let root_directory = canonical_artifact_directory_node(ArtifactDirectoryNodeV1 {
            version: ARTIFACT_DIRECTORY_NODE_VERSION,
            entries: root_entries,
        })
        .unwrap();
        let (root_directory_id, _) =
            encode_artifact_directory_node(root_directory.clone()).unwrap();
        let logical_bytes = file_count * file_bytes.len() as u64;
        db.put_artifact_cas_object(
            &root_directory_id.0,
            ARTIFACT_DIRECTORY_NODE_KIND,
            ARTIFACT_DIRECTORY_NODE_VERSION,
            logical_bytes,
            &root_directory,
        )
        .unwrap();
        let tree = ArtifactTreeRootV1 {
            version: ARTIFACT_TREE_ROOT_VERSION,
            root_directory_id,
            logical_bytes,
            entry_count,
            path_normalizer: "trail-paths/v1".into(),
        };
        let (tree_id, _) = encode_artifact_tree_root(tree.clone()).unwrap();
        db.put_artifact_cas_object(
            &tree_id.0,
            ARTIFACT_TREE_ROOT_KIND,
            ARTIFACT_TREE_ROOT_VERSION,
            logical_bytes,
            &tree,
        )
        .unwrap();
        (tree_id, tree)
    }

    #[test]
    fn large_artifact_multi_lane_scale_acceptance() {
        if std::env::var_os("TRAIL_RUN_ARTIFACT_SCALE_TEST").is_none() {
            return;
        }
        let entry_count = std::env::var("TRAIL_SCALE_ARTIFACT_ENTRIES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .unwrap_or(1_000_000);
        let lane_count = std::env::var("TRAIL_SCALE_LANES")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(20);
        assert!(matches!(entry_count, 10_000 | 100_000 | 1_000_000));
        assert!(matches!(lane_count, 1 | 5 | 20));

        let (_workspace, mut db, source_root) = initialized_resolution_fixture();
        let object_count_before = db
            .conn
            .query_row("SELECT COUNT(*) FROM artifact_objects", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap();
        let build_started = Instant::now();
        let (tree_id, tree) = synthetic_artifact_tree(&db, entry_count);
        let artifact_build_ms = build_started.elapsed().as_millis();

        let publish_started = Instant::now();
        let desired_key =
            artifact_desired_key_v2(fixture_desired_material(source_root.clone())).unwrap();
        let (envelope_id, quarantined) = db
            .put_artifact_envelope_under_write_lock(ArtifactEnvelopeV1 {
                version: ARTIFACT_ENVELOPE_VERSION,
                desired_identity: ArtifactDesiredIdentityV1::ArtifactDesiredV2 {
                    desired_key: desired_key.clone(),
                },
                tree_root_id: tree_id.clone(),
                component_id: "scale:synthetic".into(),
                output_name: "large-artifact".into(),
                output_policy: EnvironmentOutputPolicy::ImmutableSeedPrivate,
                portability_scope: "workspace".into(),
                trust_scope: "builtin".into(),
                secret_taint: ArtifactSecretTaintV1::Clear,
                resolution_snapshot_id: None,
                validation_receipt_ids: Vec::new(),
            })
            .unwrap();
        assert!(!quarantined);
        let envelope_publish_ms = publish_started.elapsed().as_millis();
        let reachability = db.artifact_content_reachability(&envelope_id).unwrap();
        let object_count_after_publish = db
            .conn
            .query_row("SELECT COUNT(*) FROM artifact_objects", [], |row| {
                row.get::<_, u64>(0)
            })
            .unwrap();

        let mode = if cfg!(target_os = "macos") {
            LaneWorkdirMode::NfsCow
        } else if cfg!(target_os = "windows") {
            LaneWorkdirMode::DokanCow
        } else {
            LaneWorkdirMode::FuseCow
        };
        let attach_started = Instant::now();
        let mut lanes = Vec::with_capacity(lane_count);
        for index in 0..lane_count {
            let lane = format!("artifact-scale-{lane_count}-{index:02}");
            db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                &lane,
                Some("main"),
                mode.clone(),
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap();
            let view = db.lane_workspace_view(&lane).unwrap().unwrap();
            let generation_id = format!("envgen_artifact_scale_{lane_count}_{index}");
            db.conn
                .execute(
                    "INSERT INTO environment_generations(
                         generation_id,view_id,generation_sequence,source_root,
                         specification_digest,predecessor_generation_id,state,created_at,
                         activated_at,retired_at)
                     VALUES(?1,?2,1,?3,'artifact-scale',NULL,'active',1,1,NULL)",
                    params![generation_id, view.view_id, source_root.0],
                )
                .unwrap();
            db.conn
                .execute(
                    "INSERT INTO environment_view_generations(view_id,generation_id,updated_at)
                     VALUES(?1,?2,1)",
                    params![view.view_id, generation_id],
                )
                .unwrap();
            db.conn
                .execute(
                    "INSERT INTO artifact_generation_bindings(
                         binding_id,generation_id,component_id,output_name,desired_key,
                         envelope_id,tree_root_id,binding_identity,created_at)
                     VALUES(?1,?2,'scale:synthetic','large-artifact',?3,?4,?5,?6,1)",
                    params![
                        format!("binding_artifact_scale_{lane_count}_{index}"),
                        generation_id,
                        desired_key.0,
                        envelope_id.0,
                        tree_id.0,
                        format!("identity_artifact_scale_{lane_count}_{index}"),
                    ],
                )
                .unwrap();
            lanes.push((lane, PathBuf::from(view.generated_upper)));
        }
        let lane_attach_ms = attach_started.elapsed().as_millis();

        let private_write_started = Instant::now();
        for (index, (_, generated_upper)) in lanes.iter().enumerate() {
            fs::write(
                generated_upper.join("lane-private.bin"),
                vec![index as u8; 4_096 + index],
            )
            .unwrap();
        }
        let private_write_ms = private_write_started.elapsed().as_millis();

        let accounting_started = Instant::now();
        let workspace_accounting = db.artifact_storage_accounting(None, 0, 0, 0, 0).unwrap();
        let representative_lane = db.lane_workspace_space(&lanes[0].0).unwrap();
        let accounting_ms = accounting_started.elapsed().as_millis();
        let lane_private_bytes = lanes
            .iter()
            .map(|(_, generated_upper)| {
                super::workspace_layer::layer_physical_bytes(generated_upper).unwrap()
            })
            .collect::<Vec<_>>();
        assert!(lane_private_bytes.iter().all(|bytes| *bytes > 0));
        assert_eq!(
            representative_lane.artifact_storage.logical_bytes,
            tree.logical_bytes
        );
        assert_eq!(
            representative_lane
                .artifact_storage
                .unique_authoritative_bytes,
            workspace_accounting.unique_authoritative_bytes
        );
        assert_eq!(representative_lane.artifact_storage.materialized_bytes, 0);
        let (binding_count, distinct_tree_count) = db
            .conn
            .query_row(
                "SELECT COUNT(*),COUNT(DISTINCT tree_root_id)
                 FROM artifact_generation_bindings",
                [],
                |row| Ok((row.get::<_, u64>(0)?, row.get::<_, u64>(1)?)),
            )
            .unwrap();
        assert_eq!(binding_count, lane_count as u64);
        assert_eq!(distinct_tree_count, 1);
        assert_eq!(workspace_accounting.logical_bytes, tree.logical_bytes);
        assert_eq!(workspace_accounting.materialized_bytes, 0);
        assert!(workspace_accounting.unique_authoritative_bytes < tree.logical_bytes);
        let materialization_count = db
            .conn
            .query_row(
                "SELECT COUNT(*) FROM artifact_materializations",
                [],
                |row| row.get::<_, u64>(0),
            )
            .unwrap();
        assert_eq!(materialization_count, 0);

        let skipped_native_gates = vec!["nfs_macos", "fuse_linux", "dokan_windows"];
        let private_total = lane_private_bytes.iter().copied().sum::<u64>();
        let evidence = serde_json::json!({
            "schema": "trail.artifact-lane-scale/v1",
            "host": {
                "platform": std::env::consts::OS,
                "architecture": std::env::consts::ARCH,
            },
            "backend": "cas-lazy-unmounted",
            "backend_qualified": true,
            "native_backend_qualified": false,
            "artifact_entries": entry_count,
            "lanes": lane_count,
            "phase_latencies_ms": {
                "artifact_build": artifact_build_ms,
                "envelope_publish": envelope_publish_ms,
                "lane_attach": lane_attach_ms,
                "private_write": private_write_ms,
                "accounting": accounting_ms,
            },
            "content_reuse": {
                "logical_bytes": tree.logical_bytes,
                "authoritative_encoded_bytes": workspace_accounting.unique_authoritative_bytes,
                "reused_logical_bytes": tree.logical_bytes.saturating_sub(workspace_accounting.unique_authoritative_bytes),
                "shared_tree_roots": distinct_tree_count,
                "lane_bindings": binding_count,
            },
            "materialization_amplification": {
                "materialization_count": materialization_count,
                "materialized_physical_bytes": workspace_accounting.materialized_bytes,
                "naive_per_lane_logical_bytes": tree.logical_bytes.saturating_mul(lane_count as u64),
                "copied_bytes": 0,
                "projected_bytes": 0,
                "prefetched_bytes": 0,
            },
            "private_deltas": {
                "lane_count": lane_private_bytes.len(),
                "total_physical_bytes": private_total,
                "minimum_physical_bytes": lane_private_bytes.iter().min(),
                "maximum_physical_bytes": lane_private_bytes.iter().max(),
            },
            "object_count": {
                "before": object_count_before,
                "after_publish": object_count_after_publish,
                "published": object_count_after_publish.saturating_sub(object_count_before),
                "reachable": reachability.object_count,
            },
            "skipped_native_gates": skipped_native_gates,
        });
        println!("{evidence}");
        if let Some(directory) = std::env::var_os("TRAIL_SCALE_EVIDENCE_DIR") {
            let directory = PathBuf::from(directory);
            fs::create_dir_all(&directory).unwrap();
            fs::write(
                directory.join(format!("artifacts-{entry_count}-lanes-{lane_count}.json")),
                serde_json::to_vec_pretty(&evidence).unwrap(),
            )
            .unwrap();
        }
    }

    #[test]
    fn object_gc_traces_resolution_snapshots_and_live_attempt_evidence() {
        let (_workspace, mut db, source_root) = initialized_resolution_fixture();
        let plan = executable_fixture_plan(&db, source_root);
        let (fence, attempt) = db.begin_artifact_resolution_attempt(plan.clone()).unwrap();
        let (snapshot_id, snapshot) = db
            .put_artifact_resolution_snapshot(
                plan,
                b"version = 4\n".to_vec(),
                BTreeMap::new(),
                BTreeMap::new(),
                vec!["index.crates.io:443".into()],
                false,
            )
            .unwrap();

        assert_eq!(db.gc(false).unwrap().pruned_objects, 0);
        for object_id in [
            attempt.plan_object_id.0,
            snapshot_id.0,
            snapshot.content_object_id.0,
        ] {
            assert_eq!(
                db.conn
                    .query_row(
                        "SELECT COUNT(*) FROM objects WHERE object_id=?1",
                        params![object_id],
                        |row| row.get::<_, u64>(0),
                    )
                    .unwrap(),
                1
            );
        }
        db.cancel_artifact_resolution_attempt(&fence.attempt_id)
            .unwrap();
    }
}
