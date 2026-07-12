use super::*;
use crate::agent_hooks::{
    parse_agent_hook_payload, AgentHookInstallPlan, AgentHookInstallScope,
    AgentHookInstallationRecord, AgentHookParseContext, AgentProviderRegistry,
};

const AGENT_HOOK_RECEIPT_OBJECT_KIND: &str = "AgentHookReceipt";
const AGENT_HOOK_RECEIPT_OBJECT_VERSION: u16 = 1;

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
struct AgentHookReceiptObject {
    version: u16,
    provider: String,
    native_event: String,
    payload_digest: String,
    redaction_profile: String,
    payload: serde_json::Value,
}

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
struct LaneArtifactObject {
    version: u16,
    artifact_kind: String,
    format: String,
    content_digest: String,
    content: Vec<u8>,
}

impl Trail {
    /// Persist ownership only after the corresponding filesystem mutation succeeds.
    pub fn record_agent_hook_installation(
        &mut self,
        plan: &AgentHookInstallPlan,
        lane: Option<&str>,
    ) -> Result<AgentHookInstallationRecord> {
        let _lock = self.acquire_write_lock()?;
        let lane_id = lane
            .map(|lane| self.lane_branch(lane).map(|branch| branch.lane_id))
            .transpose()?;
        let now = now_millis();
        let inventory = serde_json::to_string(&plan.ownership_inventory)?;
        self.conn.execute(
            "INSERT INTO agent_hook_installations
             (installation_id, workspace_id, provider, scope, config_path, lane_id,
              manifest_digest, manifest_signature_json, ownership_inventory_json,
              config_before_digest, config_after_digest, adapter_version,
              provider_version_range, detected_provider_version, capability_status,
              status, installed_at, verified_at, last_receipt_at, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, NULL, ?8, ?9, ?10, ?11,
                     NULL, NULL, 'configured', 'installed', ?12, ?12, NULL, NULL)
             ON CONFLICT(installation_id) DO UPDATE SET
               provider = excluded.provider,
               scope = excluded.scope,
               config_path = excluded.config_path,
               lane_id = excluded.lane_id,
               manifest_digest = excluded.manifest_digest,
               ownership_inventory_json = excluded.ownership_inventory_json,
               config_before_digest = excluded.config_before_digest,
               config_after_digest = excluded.config_after_digest,
               adapter_version = excluded.adapter_version,
               capability_status = excluded.capability_status,
               status = 'installed',
               installed_at = excluded.installed_at,
               verified_at = excluded.verified_at",
            params![
                plan.installation_id,
                self.config.workspace.id.0,
                plan.provider,
                plan.scope.as_str(),
                plan.config_path.to_string_lossy(),
                lane_id,
                plan.manifest_digest,
                inventory,
                plan.before_digest,
                plan.after_digest,
                plan.adapter_version,
                now,
            ],
        )?;
        self.agent_hook_installation(&plan.installation_id)
    }

    pub fn agent_hook_installation(
        &self,
        installation_id: &str,
    ) -> Result<AgentHookInstallationRecord> {
        self.conn
            .query_row(
                AGENT_HOOK_INSTALLATION_BY_ID_SELECT,
                params![installation_id, self.config.workspace.id.0],
                map_agent_hook_installation,
            )
            .optional()?
            .ok_or_else(|| Error::ObjectNotFound {
                kind: "agent hook installation",
                id: installation_id.to_string(),
            })
    }

    pub fn list_agent_hook_installations(
        &self,
        provider: Option<&str>,
    ) -> Result<Vec<AgentHookInstallationRecord>> {
        let sql = if provider.is_some() {
            AGENT_HOOK_INSTALLATION_BY_PROVIDER_SELECT
        } else {
            AGENT_HOOK_INSTALLATION_LIST_SELECT
        };
        let mut statement = self.conn.prepare(sql)?;
        let mut records = Vec::new();
        if let Some(provider) = provider {
            let rows = statement.query_map(
                params![self.config.workspace.id.0, provider],
                map_agent_hook_installation,
            )?;
            for row in rows {
                records.push(row?);
            }
        } else {
            let rows = statement.query_map(
                params![self.config.workspace.id.0],
                map_agent_hook_installation,
            )?;
            for row in rows {
                records.push(row?);
            }
        }
        Ok(records)
    }

    pub fn mark_agent_hook_installation_removed(
        &mut self,
        installation_id: &str,
    ) -> Result<AgentHookInstallationRecord> {
        let _lock = self.acquire_write_lock()?;
        let changed = self.conn.execute(
            "UPDATE agent_hook_installations
             SET status = 'removed', verified_at = ?2
             WHERE installation_id = ?1 AND workspace_id = ?3",
            params![installation_id, now_millis(), self.config.workspace.id.0],
        )?;
        if changed == 0 {
            return Err(Error::ObjectNotFound {
                kind: "agent hook installation",
                id: installation_id.to_string(),
            });
        }
        self.agent_hook_installation(installation_id)
    }

    pub fn begin_agent_capture_run(
        &mut self,
        input: AgentCaptureRunInput,
    ) -> Result<AgentCaptureRun> {
        let _lock = self.acquire_write_lock()?;
        validate_agent_provider(&input.owner_agent)?;
        if let Some(executor) = input.executor_agent.as_deref() {
            validate_agent_provider(executor)?;
        }
        validate_agent_capture_id("owner session id", &input.owner_session_id, 256)?;
        if let Some(work_item) = input.work_item_id.as_deref() {
            validate_agent_capture_id("work item id", work_item, 512)?;
        }
        validate_agent_capture_lease_ms(input.lease_ms)?;
        if let Some(metadata) = input.metadata_json.as_deref() {
            serde_json::from_str::<serde_json::Value>(metadata).map_err(|error| {
                Error::InvalidInput(format!("invalid capture run metadata JSON: {error}"))
            })?;
        }

        let workdir = PathBuf::from(input.workdir.trim());
        if !workdir.is_absolute() {
            return Err(Error::InvalidInput(
                "agent capture run workdir must be absolute".to_string(),
            ));
        }
        let canonical_workdir = workdir.canonicalize().map_err(|error| {
            Error::InvalidInput(format!(
                "cannot canonicalize agent capture run workdir `{}`: {error}",
                workdir.display()
            ))
        })?;
        if !canonical_workdir.starts_with(&self.workspace_root) {
            return Err(Error::InvalidInput(format!(
                "agent capture run workdir `{}` is outside workspace `{}`",
                canonical_workdir.display(),
                self.workspace_root.display()
            )));
        }
        let lane_id = input
            .lane
            .as_deref()
            .map(|lane| self.lane_branch(lane).map(|branch| branch.lane_id))
            .transpose()?;
        let now = now_millis();
        let expires_at = now.saturating_add(i64::try_from(input.lease_ms).unwrap_or(i64::MAX));
        let capture_run_id = format!(
            "capture_run_{}",
            crate::ids::short_hash(
                format!(
                    "{}:{}:{}:{}:{}",
                    self.config.workspace.id.0,
                    input.owner_agent,
                    input.owner_session_id,
                    canonical_workdir.display(),
                    now_nanos()
                )
                .as_bytes(),
                24,
            )
        );
        self.conn.execute(
            "INSERT INTO agent_capture_runs
             (capture_run_id, workspace_id, lane_id, workdir, canonical_workdir,
              owner_agent, owner_session_id, executor_agent, work_item_id, status,
              created_at, updated_at, expires_at, ended_at, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'active', ?10, ?10,
                     ?11, NULL, ?12)",
            params![
                capture_run_id,
                self.config.workspace.id.0,
                lane_id,
                workdir.to_string_lossy(),
                canonical_workdir.to_string_lossy(),
                input.owner_agent,
                input.owner_session_id,
                input.executor_agent,
                input.work_item_id,
                now,
                expires_at,
                input.metadata_json,
            ],
        )?;
        self.agent_capture_run(&capture_run_id)
    }

    pub fn agent_capture_run(&self, capture_run_id: &str) -> Result<AgentCaptureRun> {
        validate_agent_capture_id("capture run id", capture_run_id, 256)?;
        self.conn
            .query_row(
                AGENT_CAPTURE_RUN_SELECT_BY_ID,
                params![capture_run_id],
                agent_capture_run_row,
            )
            .optional()?
            .ok_or_else(|| {
                Error::InvalidInput(format!("agent capture run `{capture_run_id}` not found"))
            })
    }

    pub fn list_agent_capture_runs(
        &self,
        active_only: bool,
        limit: usize,
    ) -> Result<Vec<AgentCaptureRun>> {
        self.list_agent_capture_runs_page(active_only, 0, limit)
    }

    pub fn list_agent_capture_runs_page(
        &self,
        active_only: bool,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<AgentCaptureRun>> {
        let mut statement = self.conn.prepare(
            "SELECT capture_run_id, workspace_id, lane_id, workdir, canonical_workdir,
                    owner_agent, owner_session_id, executor_agent, work_item_id, status,
                    created_at, updated_at, expires_at, ended_at, metadata_json
             FROM agent_capture_runs
             WHERE workspace_id = ?1 AND (?2 = 0 OR status = 'active')
             ORDER BY updated_at DESC, capture_run_id DESC LIMIT ?3 OFFSET ?4",
        )?;
        let runs = statement
            .query_map(
                params![
                    self.config.workspace.id.0,
                    if active_only { 1 } else { 0 },
                    i64::try_from(limit.clamp(1, 1_000)).unwrap_or(1_000),
                    i64::try_from(offset.min(1_000_000)).unwrap_or(1_000_000)
                ],
                agent_capture_run_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(runs)
    }

    pub fn renew_agent_capture_run(
        &mut self,
        capture_run_id: &str,
        owner_agent: &str,
        owner_session_id: &str,
        lease_ms: u64,
    ) -> Result<AgentCaptureRun> {
        let _lock = self.acquire_write_lock()?;
        validate_agent_capture_id("capture run id", capture_run_id, 256)?;
        validate_agent_provider(owner_agent)?;
        validate_agent_capture_id("owner session id", owner_session_id, 256)?;
        validate_agent_capture_lease_ms(lease_ms)?;
        let now = now_millis();
        let expires_at = now.saturating_add(i64::try_from(lease_ms).unwrap_or(i64::MAX));
        let updated = self.conn.execute(
            "UPDATE agent_capture_runs SET updated_at = ?1, expires_at = ?2
             WHERE capture_run_id = ?3 AND owner_agent = ?4 AND owner_session_id = ?5
               AND status = 'active' AND expires_at > ?1",
            params![
                now,
                expires_at,
                capture_run_id,
                owner_agent,
                owner_session_id
            ],
        )?;
        if updated != 1 {
            return Err(Error::InvalidInput(format!(
                "agent capture run `{capture_run_id}` is expired, ended, or owned by another session"
            )));
        }
        self.agent_capture_run(capture_run_id)
    }

    pub fn end_agent_capture_run(
        &mut self,
        capture_run_id: &str,
        owner_agent: &str,
        owner_session_id: &str,
    ) -> Result<AgentCaptureRun> {
        let _lock = self.acquire_write_lock()?;
        validate_agent_capture_id("capture run id", capture_run_id, 256)?;
        validate_agent_provider(owner_agent)?;
        validate_agent_capture_id("owner session id", owner_session_id, 256)?;
        let now = now_millis();
        let updated = self.conn.execute(
            "UPDATE agent_capture_runs
             SET status = 'ended', updated_at = ?1, ended_at = ?1
             WHERE capture_run_id = ?2 AND owner_agent = ?3 AND owner_session_id = ?4
               AND status = 'active'",
            params![now, capture_run_id, owner_agent, owner_session_id],
        )?;
        if updated != 1 {
            let existing = self.agent_capture_run(capture_run_id)?;
            if existing.status == "ended"
                && existing.owner_agent == owner_agent
                && existing.owner_session_id == owner_session_id
            {
                return Ok(existing);
            }
            return Err(Error::InvalidInput(format!(
                "agent capture run `{capture_run_id}` is not active for this owner"
            )));
        }
        self.agent_capture_run(capture_run_id)
    }

    /// Expire abandoned managed runs and close their still-open captured work as interrupted.
    pub fn reconcile_expired_agent_capture_runs(&mut self) -> Result<AgentCaptureRecoveryReport> {
        let now = now_millis();
        let mut expired_run_ids = {
            let mut statement = self.conn.prepare(
                "SELECT capture_run_id FROM agent_capture_runs
                 WHERE workspace_id = ?1 AND status = 'active' AND expires_at <= ?2
                 ORDER BY capture_run_id",
            )?;
            let rows = statement
                .query_map(params![self.config.workspace.id.0, now], |row| row.get(0))?
                .collect::<std::result::Result<Vec<String>, _>>()?;
            rows
        };
        if !expired_run_ids.is_empty() {
            let _lock = self.acquire_write_lock()?;
            self.conn.execute(
                "UPDATE agent_capture_runs SET status = 'expired', updated_at = ?2, ended_at = ?2
                 WHERE workspace_id = ?1 AND status = 'active' AND expires_at <= ?2",
                params![self.config.workspace.id.0, now],
            )?;
        }

        let mappings = {
            let mut statement = self.conn.prepare(
                "SELECT s.mapping_id
                 FROM lane_agent_sessions s
                 JOIN agent_capture_runs r ON r.capture_run_id = s.capture_run_id
                 WHERE s.workspace_id = ?1 AND r.status = 'expired'
                   AND s.status IN ('active', 'finalizing')
                 ORDER BY s.mapping_id",
            )?;
            let rows = statement
                .query_map(params![self.config.workspace.id.0], |row| row.get(0))?
                .collect::<std::result::Result<Vec<String>, _>>()?;
            rows
        };
        let mut interrupted_mapping_ids = Vec::new();
        let mut interrupted_turn_ids = Vec::new();
        for mapping_id in mappings {
            let mapping = self.lane_agent_session(&mapping_id)?;
            let lane = self.lane_name_by_id(&mapping.lane_id)?;
            if let Some(turn_id) = self.open_turn_for_agent_session(&mapping.trail_session_id)? {
                if self.lane_details(&lane)?.branch.workdir.is_some() {
                    self.record_lane_workdir_for_turn(
                        &lane,
                        &turn_id,
                        Some("recovered interrupted agent turn".to_string()),
                    )?;
                }
                self.end_lane_turn(&turn_id, "interrupted")?;
                self.create_turn_evidence_manifest(&turn_id)?;
                interrupted_turn_ids.push(turn_id);
            }
            let session = self.lane_session(&mapping.trail_session_id)?;
            if session.status == "active" {
                self.end_lane_session(&mapping.trail_session_id, "interrupted")?;
            }
            let _lock = self.acquire_write_lock()?;
            let changed = self.conn.execute(
                "UPDATE lane_agent_sessions
                 SET status = 'interrupted', pending_turn_outcome = NULL,
                     session_close_requested = 1, finalization_owner = NULL,
                     finalization_lease_expires_at = NULL, updated_at = ?3
                 WHERE mapping_id = ?1 AND status = ?2",
                params![
                    mapping_id,
                    agent_capture_phase_name(mapping.status),
                    now_millis()
                ],
            )?;
            if changed == 1 {
                interrupted_mapping_ids.push(mapping_id);
            }
        }
        expired_run_ids.sort();
        interrupted_mapping_ids.sort();
        interrupted_turn_ids.sort();
        Ok(AgentCaptureRecoveryReport {
            expired_run_ids,
            interrupted_mapping_ids,
            interrupted_turn_ids,
        })
    }

    /// Resolve a governing run without guessing between equally specific candidates.
    pub fn match_agent_capture_run(
        &self,
        cwd: impl AsRef<Path>,
        agent: &str,
    ) -> Result<Option<AgentCaptureRun>> {
        validate_agent_provider(agent)?;
        let canonical_cwd = cwd.as_ref().canonicalize().map_err(|error| {
            Error::InvalidInput(format!(
                "cannot canonicalize capture cwd `{}`: {error}",
                cwd.as_ref().display()
            ))
        })?;
        let now = now_millis();
        let mut stmt = self.conn.prepare(
            "SELECT capture_run_id, workspace_id, lane_id, workdir, canonical_workdir,
                    owner_agent, owner_session_id, executor_agent, work_item_id, status,
                    created_at, updated_at, expires_at, ended_at, metadata_json
             FROM agent_capture_runs
             WHERE workspace_id = ?1 AND status = 'active' AND expires_at > ?2
               AND (owner_agent = ?3 OR executor_agent = ?3)
             ORDER BY canonical_workdir ASC, created_at ASC",
        )?;
        let runs = stmt
            .query_map(
                params![self.config.workspace.id.0, now, agent],
                agent_capture_run_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut candidates = runs
            .into_iter()
            .filter_map(|run| {
                let path = PathBuf::from(&run.canonical_workdir);
                canonical_cwd
                    .starts_with(&path)
                    .then_some((path.components().count(), run))
            })
            .collect::<Vec<_>>();
        candidates.sort_by(|(left_depth, left), (right_depth, right)| {
            right_depth
                .cmp(left_depth)
                .then_with(|| left.capture_run_id.cmp(&right.capture_run_id))
        });
        let Some((best_depth, best)) = candidates.first() else {
            return Ok(None);
        };
        if candidates
            .get(1)
            .is_some_and(|(second_depth, _)| second_depth == best_depth)
        {
            return Err(Error::InvalidInput(format!(
                "multiple agent capture runs equally match `{}` for `{agent}`",
                canonical_cwd.display()
            )));
        }
        Ok(Some(best.clone()))
    }

    /// Create the stable provider-native to Trail session mapping exactly once.
    pub fn ensure_lane_agent_session(
        &mut self,
        input: LaneAgentSessionInput,
    ) -> Result<LaneAgentSession> {
        let _lock = self.acquire_write_lock()?;
        validate_agent_provider(&input.provider)?;
        validate_agent_capture_id("native session id", &input.native_session_id, 1024)?;
        if let Some(parent) = input.parent_native_session_id.as_deref() {
            validate_agent_capture_id("parent native session id", parent, 1024)?;
        }
        validate_ref_segment(&input.lane)?;
        validate_agent_capture_id("Trail session id", &input.trail_session_id, 256)?;
        let branch = self.lane_branch(&input.lane)?;
        let session = self.lane_session(&input.trail_session_id)?;
        if session.lane_id != branch.lane_id {
            return Err(Error::InvalidInput(format!(
                "session `{}` does not belong to lane `{}`",
                input.trail_session_id, input.lane
            )));
        }
        if let Some(run_id) = input.capture_run_id.as_deref() {
            validate_agent_capture_id("capture run id", run_id, 256)?;
            let run: Option<(String, Option<String>, String, i64)> = self
                .conn
                .query_row(
                    "SELECT workspace_id, lane_id, status, expires_at
                     FROM agent_capture_runs WHERE capture_run_id = ?1",
                    params![run_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()?;
            let Some((run_workspace, run_lane, run_status, expires_at)) = run else {
                return Err(Error::InvalidInput(format!(
                    "agent capture run `{run_id}` not found"
                )));
            };
            if run_workspace != self.config.workspace.id.0
                || run_lane
                    .as_deref()
                    .is_some_and(|lane| lane != branch.lane_id)
                || run_status != "active"
                || expires_at <= now_millis()
            {
                return Err(Error::InvalidInput(format!(
                    "agent capture run `{run_id}` is not active for lane `{}`",
                    input.lane
                )));
            }
        }

        if let Some(existing) =
            self.try_lane_agent_session(&input.provider, &input.native_session_id)?
        {
            if existing.trail_session_id != input.trail_session_id
                || existing.lane_id != branch.lane_id
            {
                return Err(Error::InvalidInput(format!(
                    "native session `{}` is already mapped to a different Trail session",
                    input.native_session_id
                )));
            }
            // Managed runs stamp only mappings created while the run governs capture.
            // Existing direct sessions are deliberately never restamped here.
            return Ok(existing);
        }

        let workspace_id = self.config.workspace.id.0.clone();
        let mapping_id = format!(
            "agent_session_{}",
            crate::ids::short_hash(
                format!(
                    "{}:{}:{}",
                    workspace_id, input.provider, input.native_session_id
                )
                .as_bytes(),
                24,
            )
        );
        let now = now_millis();
        self.conn.execute(
            "INSERT INTO lane_agent_sessions
             (mapping_id, workspace_id, provider, native_session_id,
              parent_native_session_id, trail_session_id, lane_id, capture_run_id,
              primary_transport, transcript_identity, transcript_offset, resume_json,
              last_attestation_id, status, pending_turn_outcome,
              session_close_requested, capture_epoch, finalization_owner,
              finalization_lease_expires_at, next_receive_sequence, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL, NULL,
                     'idle', NULL, 0, 1, NULL, NULL, 1, ?11, ?11)",
            params![
                mapping_id,
                workspace_id,
                input.provider,
                input.native_session_id,
                input.parent_native_session_id,
                input.trail_session_id,
                branch.lane_id,
                input.capture_run_id,
                agent_capture_transport_name(input.primary_transport),
                input.transcript_identity,
                now,
            ],
        )?;
        self.lane_agent_session(&mapping_id)
    }

    pub fn lane_agent_session(&self, mapping_id: &str) -> Result<LaneAgentSession> {
        validate_agent_capture_id("agent session mapping id", mapping_id, 256)?;
        self.conn
            .query_row(
                LANE_AGENT_SESSION_SELECT_BY_ID,
                params![mapping_id],
                lane_agent_session_row,
            )
            .optional()?
            .ok_or_else(|| {
                Error::InvalidInput(format!("agent session mapping `{mapping_id}` not found"))
            })
    }

    pub fn try_lane_agent_session(
        &self,
        provider: &str,
        native_session_id: &str,
    ) -> Result<Option<LaneAgentSession>> {
        validate_agent_provider(provider)?;
        validate_agent_capture_id("native session id", native_session_id, 1024)?;
        self.conn
            .query_row(
                "SELECT mapping_id, workspace_id, provider, native_session_id,
                        parent_native_session_id, trail_session_id, lane_id, capture_run_id,
                        primary_transport, transcript_identity, transcript_offset, resume_json,
                        last_attestation_id, status, pending_turn_outcome,
                        session_close_requested, capture_epoch, finalization_owner,
                        finalization_lease_expires_at, next_receive_sequence, created_at, updated_at
                 FROM lane_agent_sessions
                 WHERE workspace_id = ?1 AND provider = ?2 AND native_session_id = ?3",
                params![self.config.workspace.id.0, provider, native_session_id],
                lane_agent_session_row,
            )
            .optional()
            .map_err(Error::from)
    }

    /// Resolve a provider-native session id for user-facing selectors. Native ids are
    /// normally globally unique, but fail closed if two providers claim the same id.
    pub fn try_lane_agent_session_by_native_id(
        &self,
        native_session_id: &str,
    ) -> Result<Option<LaneAgentSession>> {
        validate_agent_capture_id("native session id", native_session_id, 1024)?;
        let mut statement = self.conn.prepare(
            "SELECT mapping_id, workspace_id, provider, native_session_id,
                    parent_native_session_id, trail_session_id, lane_id, capture_run_id,
                    primary_transport, transcript_identity, transcript_offset, resume_json,
                    last_attestation_id, status, pending_turn_outcome,
                    session_close_requested, capture_epoch, finalization_owner,
                    finalization_lease_expires_at, next_receive_sequence, created_at, updated_at
             FROM lane_agent_sessions
             WHERE workspace_id = ?1 AND native_session_id = ?2
             ORDER BY updated_at DESC, mapping_id DESC
             LIMIT 2",
        )?;
        let mappings = statement
            .query_map(
                params![self.config.workspace.id.0, native_session_id],
                lane_agent_session_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        match mappings.as_slice() {
            [] => Ok(None),
            [mapping] => Ok(Some(mapping.clone())),
            _ => Err(Error::Conflict(format!(
                "native session id `{native_session_id}` is ambiguous across providers"
            ))),
        }
    }

    pub fn list_lane_agent_sessions(
        &self,
        provider: Option<&str>,
        status: Option<AgentCapturePhase>,
        limit: usize,
    ) -> Result<Vec<LaneAgentSession>> {
        self.list_lane_agent_sessions_page(provider, status, 0, limit)
    }

    pub fn list_lane_agent_sessions_page(
        &self,
        provider: Option<&str>,
        status: Option<AgentCapturePhase>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<LaneAgentSession>> {
        if let Some(provider) = provider {
            validate_agent_provider(provider)?;
        }
        let status_name = status.map(agent_capture_phase_name);
        let mut statement = self.conn.prepare(
            "SELECT mapping_id, workspace_id, provider, native_session_id,
                    parent_native_session_id, trail_session_id, lane_id, capture_run_id,
                    primary_transport, transcript_identity, transcript_offset, resume_json,
                    last_attestation_id, status, pending_turn_outcome,
                    session_close_requested, capture_epoch, finalization_owner,
                    finalization_lease_expires_at, next_receive_sequence, created_at, updated_at
             FROM lane_agent_sessions
             WHERE workspace_id = ?1 AND (?2 IS NULL OR provider = ?2)
               AND (?3 IS NULL OR status = ?3)
             ORDER BY updated_at DESC, mapping_id DESC LIMIT ?4 OFFSET ?5",
        )?;
        let rows = statement
            .query_map(
                params![
                    self.config.workspace.id.0,
                    provider,
                    status_name,
                    i64::try_from(limit.clamp(1, 1_000)).unwrap_or(1_000),
                    i64::try_from(offset.min(1_000_000)).unwrap_or(1_000_000)
                ],
                lane_agent_session_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        Ok(rows)
    }

    /// Acquire or renew the one durable finalizer lease for a native session.
    pub fn acquire_agent_finalization_lease(
        &mut self,
        mapping_id: &str,
        owner: &str,
        ttl_ms: u64,
        outcome: AgentTurnOutcome,
        request_session_close: bool,
    ) -> Result<AgentFinalizationLeaseReport> {
        let _lock = self.acquire_write_lock()?;
        validate_agent_capture_id("agent session mapping id", mapping_id, 256)?;
        validate_agent_capture_id("finalization owner", owner, 256)?;
        if !(1_000..=300_000).contains(&ttl_ms) {
            return Err(Error::InvalidInput(
                "agent finalization lease must be between 1000 and 300000 milliseconds".to_string(),
            ));
        }
        let now = now_millis();
        let expires_at = now.saturating_add(i64::try_from(ttl_ms).unwrap_or(i64::MAX));
        let updated = self.conn.execute(
            "UPDATE lane_agent_sessions
             SET status = 'finalizing', pending_turn_outcome = ?1,
                 session_close_requested = CASE
                     WHEN session_close_requested = 1 OR ?2 = 1 THEN 1 ELSE 0 END,
                 finalization_owner = ?3, finalization_lease_expires_at = ?4,
                 updated_at = ?5
             WHERE mapping_id = ?6
               AND status IN ('idle', 'active', 'finalizing')
               AND (status != 'finalizing' OR finalization_owner = ?3
                    OR finalization_lease_expires_at IS NULL
                    OR finalization_lease_expires_at <= ?5)",
            params![
                agent_turn_outcome_name(outcome),
                request_session_close,
                owner,
                expires_at,
                now,
                mapping_id,
            ],
        )?;
        let mapping = self.lane_agent_session(mapping_id)?;
        Ok(AgentFinalizationLeaseReport {
            expires_at: mapping.finalization_lease_expires_at.unwrap_or(expires_at),
            mapping,
            acquired: updated == 1,
            owner: owner.to_string(),
        })
    }

    /// Persist a bounded, redacted native receipt before semantic processing.
    pub fn persist_agent_hook_receipt(
        &mut self,
        input: AgentHookReceiptInput,
    ) -> Result<AgentHookReceiptIngestReport> {
        let _lock = self.acquire_write_lock()?;
        validate_agent_receipt_input(&input)?;

        let workspace_id = self.config.workspace.id.0.clone();
        if let Some(installation_id) = input.installation_id.as_deref() {
            let installation: Option<(String, String, String)> = self
                .conn
                .query_row(
                    "SELECT workspace_id, provider, status FROM agent_hook_installations
                     WHERE installation_id = ?1",
                    params![installation_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let Some((installation_workspace, installation_provider, installation_status)) =
                installation
            else {
                return Err(Error::InvalidInput(format!(
                    "agent hook installation `{installation_id}` not found"
                )));
            };
            if installation_workspace != workspace_id || installation_provider != input.provider {
                return Err(Error::InvalidInput(format!(
                    "agent hook installation `{installation_id}` does not authorize provider `{}` in this workspace",
                    input.provider
                )));
            }
            if installation_status != "installed" {
                return Err(Error::InvalidInput(format!(
                    "agent hook installation `{installation_id}` is `{installation_status}` and cannot ingest receipts"
                )));
            }
        }

        let redacted_payload = redact_sensitive_json(input.payload.clone());
        let payload_bytes = serde_json::to_vec(&redacted_payload)?;
        let configured_limit =
            usize::try_from(self.config.lane.max_event_payload_bytes).unwrap_or(usize::MAX);
        let payload_limit = if configured_limit == 0 {
            AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES
        } else {
            configured_limit.min(AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES)
        };
        if payload_bytes.len() > payload_limit {
            return Err(Error::InvalidInput(format!(
                "agent hook receipt payload is {} bytes; maximum is {payload_limit}",
                payload_bytes.len()
            )));
        }
        let payload_digest = format!("sha256:{}", sha256_hex(&payload_bytes));

        if let Some(existing) = self.try_agent_hook_receipt_by_dedupe_key(
            &workspace_id,
            &input.provider,
            &input.dedupe_key,
        )? {
            if existing.payload_digest != payload_digest {
                return Err(Error::InvalidInput(format!(
                    "agent hook dedupe key `{}` was reused with different payload content",
                    input.dedupe_key
                )));
            }
            return Ok(AgentHookReceiptIngestReport {
                receipt: existing,
                duplicate: true,
            });
        }

        let stored = AgentHookReceiptObject {
            version: AGENT_HOOK_RECEIPT_OBJECT_VERSION,
            provider: input.provider.clone(),
            native_event: input.native_event.clone(),
            payload_digest: payload_digest.clone(),
            redaction_profile: "trail-sensitive-json/v1".to_string(),
            payload: redacted_payload,
        };
        let raw_object_id = self.put_object(
            AGENT_HOOK_RECEIPT_OBJECT_KIND,
            AGENT_HOOK_RECEIPT_OBJECT_VERSION,
            &stored,
        )?;
        let receipt_id = format!(
            "receipt_{}",
            crate::ids::short_hash(
                format!(
                    "{}:{}:{}:{}",
                    workspace_id, input.provider, input.dedupe_key, payload_digest
                )
                .as_bytes(),
                24,
            )
        );
        let now = now_millis();
        self.conn.execute(
            "INSERT INTO agent_hook_receipts
             (receipt_id, workspace_id, installation_id, mapping_id, provider,
              native_event, native_session_id, native_turn_id, transport, dedupe_key,
              payload_digest, raw_object_id, raw_artifact_id, receive_sequence, status,
              attempt_count, next_attempt_at, diagnostic, occurred_at, received_at,
              processed_at, updated_at)
             VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, NULL,
                     NULL, 'received', 0, NULL, NULL, ?12, ?13, NULL, ?13)",
            params![
                receipt_id,
                workspace_id,
                input.installation_id,
                input.provider,
                input.native_event,
                input.native_session_id,
                input.native_turn_id,
                agent_capture_transport_name(input.transport),
                input.dedupe_key,
                payload_digest,
                raw_object_id.0,
                input.occurred_at,
                now,
            ],
        )?;
        Ok(AgentHookReceiptIngestReport {
            receipt: self.agent_hook_receipt(&receipt_id)?,
            duplicate: false,
        })
    }

    pub fn agent_hook_receipt(&self, receipt_id: &str) -> Result<AgentHookReceipt> {
        validate_agent_capture_id("receipt id", receipt_id, 256)?;
        self.conn
            .query_row(
                "SELECT receipt_id, workspace_id, installation_id, mapping_id, provider,
                        native_event, native_session_id, native_turn_id, transport, dedupe_key,
                        payload_digest, raw_object_id, raw_artifact_id, receive_sequence, status,
                        attempt_count, next_attempt_at, diagnostic, occurred_at, received_at,
                        processed_at, updated_at
                 FROM agent_hook_receipts WHERE receipt_id = ?1",
                params![receipt_id],
                agent_hook_receipt_row,
            )
            .optional()?
            .ok_or_else(|| {
                Error::InvalidInput(format!("agent hook receipt `{receipt_id}` not found"))
            })
    }

    pub fn list_agent_hook_receipts(
        &self,
        provider: Option<&str>,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<AgentHookReceipt>> {
        self.list_agent_hook_receipts_page(provider, status, 0, limit)
    }

    pub fn list_agent_hook_receipts_page(
        &self,
        provider: Option<&str>,
        status: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<AgentHookReceipt>> {
        if let Some(provider) = provider {
            validate_agent_provider(provider)?;
        }
        if let Some(status) = status {
            validate_agent_receipt_status(status)?;
        }
        let limit = limit.clamp(1, 1_000) as i64;
        let mut stmt = self.conn.prepare(
            "SELECT receipt_id, workspace_id, installation_id, mapping_id, provider,
                    native_event, native_session_id, native_turn_id, transport, dedupe_key,
                    payload_digest, raw_object_id, raw_artifact_id, receive_sequence, status,
                    attempt_count, next_attempt_at, diagnostic, occurred_at, received_at,
                    processed_at, updated_at
             FROM agent_hook_receipts
             WHERE (?1 IS NULL OR provider = ?1) AND (?2 IS NULL OR status = ?2)
             ORDER BY received_at DESC, receipt_id DESC LIMIT ?3 OFFSET ?4",
        )?;
        let receipts = stmt
            .query_map(
                params![
                    provider,
                    status,
                    limit,
                    i64::try_from(offset.min(1_000_000)).unwrap_or(1_000_000)
                ],
                agent_hook_receipt_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        Ok(receipts)
    }

    pub fn recover_stale_agent_hook_receipts(&mut self, stale_after_ms: u64) -> Result<usize> {
        if !(1_000..=86_400_000).contains(&stale_after_ms) {
            return Err(Error::InvalidInput(
                "stale receipt timeout must be between 1000 and 86400000 milliseconds".to_string(),
            ));
        }
        let now = now_millis();
        let cutoff = now.saturating_sub(i64::try_from(stale_after_ms).unwrap_or(i64::MAX));
        let _lock = self.acquire_write_lock()?;
        self.conn
            .execute(
                "UPDATE agent_hook_receipts
                 SET status = 'retry', next_attempt_at = ?1,
                     diagnostic = 'processing lease expired; receipt recovered for replay',
                     updated_at = ?1
                 WHERE workspace_id = ?2 AND status = 'processing' AND updated_at <= ?3",
                params![now, self.config.workspace.id.0, cutoff],
            )
            .map_err(Error::from)
    }

    pub fn retry_agent_hook_receipt(&mut self, receipt_id: &str) -> Result<AgentHookReceipt> {
        let existing = self.agent_hook_receipt(receipt_id)?;
        if existing.status == "received" {
            return Ok(existing);
        }
        if !matches!(existing.status.as_str(), "retry" | "quarantined") {
            return Err(Error::Conflict(format!(
                "agent hook receipt `{receipt_id}` cannot be retried from `{}`",
                existing.status
            )));
        }
        let _lock = self.acquire_write_lock()?;
        let changed = self.conn.execute(
            "UPDATE agent_hook_receipts
             SET status = 'received', next_attempt_at = NULL, diagnostic = NULL,
                 updated_at = ?2
             WHERE receipt_id = ?1 AND status IN ('retry', 'quarantined')",
            params![receipt_id, now_millis()],
        )?;
        if changed != 1 {
            return Err(Error::Conflict(format!(
                "agent hook receipt `{receipt_id}` changed while requesting retry"
            )));
        }
        self.agent_hook_receipt(receipt_id)
    }

    pub fn discard_agent_hook_receipt(&mut self, receipt_id: &str) -> Result<AgentHookReceipt> {
        let existing = self.agent_hook_receipt(receipt_id)?;
        if existing.status == "discarded" {
            return Ok(existing);
        }
        if !matches!(
            existing.status.as_str(),
            "received" | "retry" | "quarantined"
        ) {
            return Err(Error::Conflict(format!(
                "agent hook receipt `{receipt_id}` cannot be discarded from `{}`",
                existing.status
            )));
        }
        let _lock = self.acquire_write_lock()?;
        let changed = self.conn.execute(
            "UPDATE agent_hook_receipts
             SET status = 'discarded', next_attempt_at = NULL,
                 diagnostic = 'discarded explicitly by operator', updated_at = ?2
             WHERE receipt_id = ?1 AND status IN ('received', 'retry', 'quarantined')",
            params![receipt_id, now_millis()],
        )?;
        if changed != 1 {
            return Err(Error::Conflict(format!(
                "agent hook receipt `{receipt_id}` changed while discarding"
            )));
        }
        self.agent_hook_receipt(receipt_id)
    }

    pub fn replay_pending_agent_hook_receipts(
        &mut self,
        limit: usize,
    ) -> Result<AgentHookReplayBatchReport> {
        let recovered_stale_receipts = self.recover_stale_agent_hook_receipts(60_000)?;
        let now = now_millis();
        let mut statement = self.conn.prepare(
            "SELECT receipt_id FROM agent_hook_receipts
             WHERE workspace_id = ?1
               AND (status = 'received' OR
                    (status = 'retry' AND (next_attempt_at IS NULL OR next_attempt_at <= ?2)))
             ORDER BY received_at, receipt_id LIMIT ?3",
        )?;
        let receipt_ids = statement
            .query_map(
                params![
                    self.config.workspace.id.0,
                    now,
                    i64::try_from(limit.clamp(1, 1_000)).unwrap_or(1_000)
                ],
                |row| row.get::<_, String>(0),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        drop(statement);
        let mut replayed = Vec::new();
        let mut failures = Vec::new();
        for receipt_id in receipt_ids {
            match self.replay_agent_hook_receipt(&receipt_id) {
                Ok(report) => replayed.push(report),
                Err(error) => failures.push(AgentHookReplayFailure {
                    receipt_id,
                    code: error.code().to_string(),
                    message: redact_agent_capture_diagnostic(&error.to_string()),
                }),
            }
        }
        Ok(AgentHookReplayBatchReport {
            recovered_stale_receipts,
            replayed,
            failures,
        })
    }

    pub fn record_lane_artifact(&mut self, input: LaneArtifactInput) -> Result<LaneArtifact> {
        const MAX_ARTIFACT_BYTES: usize = 64 * 1024 * 1024;
        if input.content.len() > MAX_ARTIFACT_BYTES {
            return Err(Error::InvalidInput(format!(
                "agent artifact is {} bytes; maximum is {MAX_ARTIFACT_BYTES}",
                input.content.len()
            )));
        }
        validate_agent_provider(&input.provider)?;
        for (name, value) in [
            ("artifact kind", input.artifact_kind.as_str()),
            ("artifact format", input.format.as_str()),
            ("artifact trust", input.trust.as_str()),
        ] {
            validate_agent_capture_id(name, value, 128)?;
        }
        if let Some(metadata) = input.metadata_json.as_deref() {
            serde_json::from_str::<serde_json::Value>(metadata).map_err(|error| {
                Error::InvalidInput(format!("invalid agent artifact metadata JSON: {error}"))
            })?;
        }
        if input
            .start_offset
            .zip(input.end_offset)
            .is_some_and(|(start, end)| start > end)
        {
            return Err(Error::InvalidInput(
                "agent artifact start_offset exceeds end_offset".to_string(),
            ));
        }
        let branch = self.lane_branch(&input.lane)?;
        let session = self.lane_session(&input.session_id)?;
        if session.lane_id != branch.lane_id {
            return Err(Error::InvalidInput(format!(
                "session `{}` does not belong to lane `{}`",
                input.session_id, input.lane
            )));
        }
        if let Some(turn_id) = input.turn_id.as_deref() {
            let turn = self.lane_turn(turn_id)?;
            if turn.session_id.as_deref() != Some(input.session_id.as_str()) {
                return Err(Error::InvalidInput(format!(
                    "turn `{turn_id}` does not belong to session `{}`",
                    input.session_id
                )));
            }
        }
        if let Some(supersedes) = input.supersedes_artifact_id.as_deref() {
            self.lane_artifact(supersedes)?;
        }
        let content_digest = format!("sha256:{}", sha256_hex(&input.content));
        let object = LaneArtifactObject {
            version: 1,
            artifact_kind: input.artifact_kind.clone(),
            format: input.format.clone(),
            content_digest: content_digest.clone(),
            content: input.content.clone(),
        };
        let object_id = self.put_object("LaneArtifact", 1, &object)?;
        let now = now_millis();
        let artifact_id = format!(
            "artifact_{}",
            crate::ids::short_hash(
                format!(
                    "{}:{}:{}:{}:{}:{}",
                    self.config.workspace.id.0,
                    input.session_id,
                    input.turn_id.as_deref().unwrap_or("session"),
                    input.artifact_kind,
                    content_digest,
                    input.start_offset.unwrap_or(0)
                )
                .as_bytes(),
                24,
            )
        );
        let _lock = self.acquire_write_lock()?;
        self.conn.execute(
            "INSERT INTO lane_artifacts
             (artifact_id, workspace_id, lane_id, session_id, turn_id, provider,
              artifact_kind, format, source, source_locator_redacted, content_object_id,
              content_digest, size_bytes, start_offset, end_offset, redaction_profile,
              retention_status, trust, supersedes_artifact_id, created_at, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, 'active', ?17, ?18, ?19, ?20)
             ON CONFLICT(artifact_id) DO NOTHING",
            params![
                artifact_id,
                self.config.workspace.id.0,
                branch.lane_id,
                input.session_id,
                input.turn_id,
                input.provider,
                input.artifact_kind,
                input.format,
                agent_evidence_source_name(input.source),
                input
                    .source_locator_redacted
                    .as_deref()
                    .map(redact_sensitive_text),
                object_id.0,
                content_digest,
                i64::try_from(input.content.len()).unwrap_or(i64::MAX),
                input
                    .start_offset
                    .and_then(|value| i64::try_from(value).ok()),
                input.end_offset.and_then(|value| i64::try_from(value).ok()),
                input.redaction_profile,
                input.trust,
                input.supersedes_artifact_id,
                now,
                input.metadata_json,
            ],
        )?;
        self.lane_artifact(&artifact_id)
    }

    pub fn lane_artifact(&self, artifact_id: &str) -> Result<LaneArtifact> {
        self.conn
            .query_row(
                LANE_ARTIFACT_BY_ID_SELECT,
                params![artifact_id],
                map_lane_artifact,
            )
            .optional()?
            .ok_or_else(|| Error::ObjectNotFound {
                kind: "lane artifact",
                id: artifact_id.to_string(),
            })
    }

    pub fn lane_artifact_content(&self, artifact_id: &str) -> Result<Vec<u8>> {
        let artifact = self.lane_artifact(artifact_id)?;
        let object_id =
            artifact
                .content_object_id
                .as_ref()
                .ok_or_else(|| Error::ObjectNotFound {
                    kind: "lane artifact content",
                    id: artifact_id.to_string(),
                })?;
        let object: LaneArtifactObject = self.get_object("LaneArtifact", object_id)?;
        if object.content_digest != artifact.content_digest
            || object.artifact_kind != artifact.artifact_kind
            || object.format != artifact.format
            || format!("sha256:{}", sha256_hex(&object.content)) != artifact.content_digest
        {
            return Err(Error::Corrupt(format!(
                "lane artifact `{artifact_id}` content does not match its immutable metadata"
            )));
        }
        Ok(object.content)
    }

    /// Remove attachment access while preserving immutable digest and evidence identity.
    pub fn redact_lane_artifact(
        &mut self,
        artifact_id: &str,
        reason: &str,
    ) -> Result<LaneArtifact> {
        validate_agent_capture_id("artifact redaction reason", reason, 512)?;
        let existing = self.lane_artifact(artifact_id)?;
        if existing.retention_status == "redacted" {
            return Ok(existing);
        }
        {
            let _lock = self.acquire_write_lock()?;
            let changed = self.conn.execute(
                "UPDATE lane_artifacts
                 SET content_object_id = NULL, retention_status = 'redacted'
                 WHERE artifact_id = ?1 AND retention_status = 'active'",
                params![artifact_id],
            )?;
            if changed != 1 {
                return Err(Error::Conflict(format!(
                    "agent artifact `{artifact_id}` changed while redacting attachment access"
                )));
            }
        }
        let lane = self.lane_name_by_id(&existing.lane_id)?;
        self.add_lane_session_event(
            &lane,
            &existing.session_id,
            "artifact.redacted",
            Some(serde_json::json!({
                "artifact_id": artifact_id,
                "content_digest": existing.content_digest,
                "reason": redact_agent_capture_diagnostic(reason),
            })),
        )?;
        self.lane_artifact(artifact_id)
    }

    pub fn list_lane_artifacts(
        &self,
        session_id: &str,
        turn_id: Option<&str>,
        limit: usize,
    ) -> Result<Vec<LaneArtifact>> {
        self.list_lane_artifacts_page(session_id, turn_id, 0, limit)
    }

    pub fn list_lane_artifacts_page(
        &self,
        session_id: &str,
        turn_id: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<Vec<LaneArtifact>> {
        self.lane_session(session_id)?;
        let mut statement = self.conn.prepare(
            "SELECT artifact_id, workspace_id, lane_id, session_id, turn_id, provider,
                    artifact_kind, format, source, source_locator_redacted, content_object_id,
                    content_digest, size_bytes, start_offset, end_offset, redaction_profile,
                    retention_status, trust, supersedes_artifact_id, created_at, metadata_json
             FROM lane_artifacts
             WHERE session_id = ?1 AND (?2 IS NULL OR turn_id = ?2)
             ORDER BY created_at DESC, artifact_id DESC LIMIT ?3 OFFSET ?4",
        )?;
        let artifacts = statement
            .query_map(
                params![
                    session_id,
                    turn_id,
                    i64::try_from(limit.clamp(1, 1_000)).unwrap_or(1_000),
                    i64::try_from(offset.min(1_000_000)).unwrap_or(1_000_000)
                ],
                map_lane_artifact,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Error::from)?;
        Ok(artifacts)
    }

    /// Replay one durable receipt through the provider parser and shared lifecycle machine.
    pub fn replay_agent_hook_receipt(&mut self, receipt_id: &str) -> Result<AgentHookReplayReport> {
        let receipt = self.agent_hook_receipt(receipt_id)?;
        if receipt.status == "processed" {
            return self.refresh_processed_agent_hook_receipt_artifacts(receipt);
        }
        {
            let _lock = self.acquire_write_lock()?;
            let changed = self.conn.execute(
                "UPDATE agent_hook_receipts
                 SET status = 'processing', attempt_count = attempt_count + 1,
                     diagnostic = NULL, updated_at = ?2
                 WHERE receipt_id = ?1 AND status IN ('received', 'retry')",
                params![receipt_id, now_millis()],
            )?;
            if changed == 0 {
                return Err(Error::Conflict(format!(
                    "agent hook receipt `{receipt_id}` is not replayable from status `{}`",
                    receipt.status
                )));
            }
        }

        match self.replay_agent_hook_receipt_inner(receipt_id) {
            Ok(report) => Ok(report),
            Err(error) => {
                let status = if matches!(error, Error::InvalidInput(_) | Error::Conflict(_)) {
                    "quarantined"
                } else {
                    "retry"
                };
                let diagnostic = redact_agent_capture_diagnostic(&error.to_string());
                let attempt_count = self.agent_hook_receipt(receipt_id)?.attempt_count.max(1);
                let exponent = attempt_count.saturating_sub(1).min(8);
                let backoff_ms = 1_000_i64.saturating_mul(1_i64 << exponent).min(300_000);
                let _lock = self.acquire_write_lock()?;
                self.conn.execute(
                    "UPDATE agent_hook_receipts
                     SET status = ?2, diagnostic = ?3,
                         next_attempt_at = CASE WHEN ?2 = 'retry' THEN ?4 ELSE NULL END,
                         updated_at = ?5
                     WHERE receipt_id = ?1",
                    params![
                        receipt_id,
                        status,
                        diagnostic,
                        now_millis().saturating_add(backoff_ms),
                        now_millis(),
                    ],
                )?;
                Err(error)
            }
        }
    }

    fn refresh_processed_agent_hook_receipt_artifacts(
        &mut self,
        receipt: AgentHookReceipt,
    ) -> Result<AgentHookReplayReport> {
        let Some(mapping_id) = receipt.mapping_id.clone() else {
            return Ok(AgentHookReplayReport {
                receipt,
                mapping: None,
                normalized_events: Vec::new(),
                actions: Vec::new(),
                diagnostics: Vec::new(),
                replayed: false,
            });
        };
        let mapping = self.lane_agent_session(&mapping_id)?;
        let object: AgentHookReceiptObject =
            self.get_object(AGENT_HOOK_RECEIPT_OBJECT_KIND, &receipt.raw_object_id)?;
        let registry = AgentProviderRegistry::built_in()?;
        let events = parse_agent_hook_payload(
            &registry,
            &receipt.provider,
            &receipt.native_event,
            &object.payload,
            AgentHookParseContext {
                receipt_id: receipt.receipt_id.clone(),
                workspace_id: receipt.workspace_id.clone(),
                lane_id: Some(mapping.lane_id.clone()),
                capture_run_id: mapping.capture_run_id.clone(),
                provider_version: None,
                raw_digest: receipt.payload_digest.clone(),
                received_at: receipt.received_at,
                transport: receipt.transport,
            },
        )?;
        let mut actions = Vec::new();
        for event in &events {
            if event.event_type.kind().terminal_outcome().is_some()
                || event.event_type.kind() == AgentLifecycleEventKind::SessionEnded
            {
                self.import_agent_transcript_snapshot(&mapping, event)?;
                actions.push(AgentCaptureAction::ImportTranscript);
            }
        }
        Ok(AgentHookReplayReport {
            receipt,
            mapping: Some(self.lane_agent_session(&mapping_id)?),
            normalized_events: events,
            actions,
            diagnostics: Vec::new(),
            replayed: false,
        })
    }

    fn replay_agent_hook_receipt_inner(
        &mut self,
        receipt_id: &str,
    ) -> Result<AgentHookReplayReport> {
        let receipt = self.agent_hook_receipt(receipt_id)?;
        let object: AgentHookReceiptObject =
            self.get_object(AGENT_HOOK_RECEIPT_OBJECT_KIND, &receipt.raw_object_id)?;
        if object.payload_digest != receipt.payload_digest || object.provider != receipt.provider {
            return Err(Error::Corrupt(format!(
                "agent hook receipt `{receipt_id}` object does not match its journal row"
            )));
        }
        let mapping = self.ensure_mapping_for_agent_hook_receipt(&receipt, &object.payload)?;
        let receipt = self.assign_agent_hook_receipt_mapping(receipt_id, &mapping.mapping_id)?;
        let registry = AgentProviderRegistry::built_in()?;
        let events = parse_agent_hook_payload(
            &registry,
            &receipt.provider,
            &receipt.native_event,
            &object.payload,
            AgentHookParseContext {
                receipt_id: receipt.receipt_id.clone(),
                workspace_id: receipt.workspace_id.clone(),
                lane_id: Some(mapping.lane_id.clone()),
                capture_run_id: mapping.capture_run_id.clone(),
                provider_version: None,
                raw_digest: receipt.payload_digest.clone(),
                received_at: receipt.received_at,
                transport: receipt.transport,
            },
        )?;
        let lane_name = self.lane_name_by_id(&mapping.lane_id)?;
        let mut actions = Vec::new();
        let mut diagnostics = Vec::new();
        let mut current = self.lane_agent_session(&mapping.mapping_id)?;
        for event in &events {
            if current.primary_transport == AgentCaptureTransport::Hybrid
                && self.hybrid_acp_owns_session(&current.trail_session_id)?
            {
                let mut hybrid_actions = vec![AgentCaptureAction::AppendEvidence];
                if event.event_type.kind().terminal_outcome().is_some()
                    || event.event_type.kind() == AgentLifecycleEventKind::SessionEnded
                {
                    hybrid_actions.push(AgentCaptureAction::ImportTranscript);
                }
                self.execute_agent_capture_actions(&lane_name, &current, event, &hybrid_actions)?;
                actions.extend(hybrid_actions);
                continue;
            }
            let open_turn = self.open_turn_for_agent_session(&current.trail_session_id)?;
            let already_recorded = self.agent_lifecycle_event_already_recorded(
                &current.trail_session_id,
                &event.event_id,
            )?;
            if already_recorded
                && event.event_type.kind().terminal_outcome().is_some()
                && open_turn.is_none()
                && matches!(
                    current.status,
                    AgentCapturePhase::Idle
                        | AgentCapturePhase::Ended
                        | AgentCapturePhase::Interrupted
                )
            {
                continue;
            }
            let transition = AgentCaptureCoordinator::decide(
                current.status,
                event.event_type.kind(),
                AgentTransitionContext {
                    has_session: true,
                    has_open_turn: open_turn.is_some(),
                    duplicate: false,
                    has_new_evidence: true,
                    session_close_requested: current.session_close_requested,
                    pending_turn_outcome: current.pending_turn_outcome,
                },
            );
            diagnostics.extend(transition.diagnostics.clone());
            let replay_actions = transition
                .actions
                .iter()
                .filter(|action| {
                    !(already_recorded && matches!(action, AgentCaptureAction::AppendEvidence))
                })
                .cloned()
                .collect::<Vec<_>>();
            self.execute_agent_capture_actions(&lane_name, &current, event, &replay_actions)?;
            actions.extend(transition.actions.clone());
            let persisted = self.lane_agent_session(&current.mapping_id)?;
            self.update_agent_mapping_phase(
                &current.mapping_id,
                persisted.status,
                persisted.finalization_owner.as_deref(),
                transition.new_phase,
                transition
                    .actions
                    .iter()
                    .find_map(|action| match action {
                        AgentCaptureAction::RequestTurnFinalization { outcome } => Some(*outcome),
                        _ => None,
                    })
                    .or(current.pending_turn_outcome),
                transition
                    .actions
                    .iter()
                    .any(|action| matches!(action, AgentCaptureAction::RequestSessionClose))
                    || current.session_close_requested,
            )?;
            current = self.lane_agent_session(&current.mapping_id)?;

            if transition.new_phase == AgentCapturePhase::Finalizing {
                let completed = AgentCaptureCoordinator::decide(
                    AgentCapturePhase::Finalizing,
                    AgentLifecycleEventKind::FinalizationCompleted,
                    AgentTransitionContext {
                        has_session: true,
                        has_open_turn: self
                            .open_turn_for_agent_session(&current.trail_session_id)?
                            .is_some(),
                        duplicate: false,
                        has_new_evidence: false,
                        session_close_requested: current.session_close_requested,
                        pending_turn_outcome: current.pending_turn_outcome,
                    },
                );
                self.execute_agent_capture_actions(
                    &lane_name,
                    &current,
                    event,
                    &completed.actions,
                )?;
                actions.extend(completed.actions.clone());
                diagnostics.extend(completed.diagnostics);
                let persisted = self.lane_agent_session(&current.mapping_id)?;
                self.update_agent_mapping_phase(
                    &current.mapping_id,
                    persisted.status,
                    persisted.finalization_owner.as_deref(),
                    completed.new_phase,
                    None,
                    current.session_close_requested,
                )?;
                current = self.lane_agent_session(&current.mapping_id)?;
            }
        }
        let now = now_millis();
        {
            let _lock = self.acquire_write_lock()?;
            self.conn.execute(
                "UPDATE agent_hook_receipts
                 SET status = 'processed', processed_at = ?2, next_attempt_at = NULL,
                     diagnostic = NULL, updated_at = ?2
                 WHERE receipt_id = ?1 AND status = 'processing'",
                params![receipt_id, now],
            )?;
            if let Some(installation_id) = receipt.installation_id.as_deref() {
                self.conn.execute(
                    "UPDATE agent_hook_installations
                     SET last_receipt_at = ?2, verified_at = ?2
                     WHERE installation_id = ?1",
                    params![installation_id, now],
                )?;
            }
        }
        Ok(AgentHookReplayReport {
            receipt: self.agent_hook_receipt(receipt_id)?,
            mapping: Some(current),
            normalized_events: events,
            actions,
            diagnostics,
            replayed: true,
        })
    }

    fn ensure_mapping_for_agent_hook_receipt(
        &mut self,
        receipt: &AgentHookReceipt,
        payload: &serde_json::Value,
    ) -> Result<LaneAgentSession> {
        let native_session_id = receipt.native_session_id.as_deref().ok_or_else(|| {
            Error::InvalidInput(format!(
                "agent hook receipt `{}` has no native session id",
                receipt.receipt_id
            ))
        })?;
        if let Some(mapping) = self.try_lane_agent_session(&receipt.provider, native_session_id)? {
            return Ok(mapping);
        }
        let capture_run =
            payload_string_value(payload, &["cwd", "workspaceRoot", "workspace_root"])
                .map(|cwd| self.match_agent_capture_run(cwd, &receipt.provider))
                .transpose()?
                .flatten();
        if let Some((lane_id, trail_session_id)) =
            self.match_acp_session_for_native_id(&receipt.provider, native_session_id)?
        {
            let lane = self.lane_name_by_id(&lane_id)?;
            return self.ensure_lane_agent_session(LaneAgentSessionInput {
                provider: receipt.provider.clone(),
                native_session_id: native_session_id.to_string(),
                parent_native_session_id: payload_string_value(
                    payload,
                    &["parent_session_id", "parentSessionId"],
                )
                .map(str::to_string),
                lane,
                trail_session_id,
                capture_run_id: capture_run.map(|run| run.capture_run_id),
                primary_transport: AgentCaptureTransport::Hybrid,
                transcript_identity: payload_string_value(
                    payload,
                    &["transcript_path", "transcriptPath"],
                )
                .map(str::to_string),
            });
        }
        let installation_lane_id = receipt
            .installation_id
            .as_deref()
            .map(|installation_id| {
                self.conn.query_row(
                    "SELECT lane_id FROM agent_hook_installations WHERE installation_id = ?1",
                    params![installation_id],
                    |row| row.get::<_, Option<String>>(0),
                )
            })
            .transpose()?
            .flatten();
        let lane_id = installation_lane_id.or_else(|| {
            capture_run
                .as_ref()
                .and_then(|capture_run| capture_run.lane_id.clone())
        });
        let lane = if let Some(lane_id) = lane_id.as_deref() {
            self.lane_name_by_id(lane_id)?
        } else {
            let lane = format!("agent-{}", receipt.provider);
            if self.lane_branch(&lane).is_err() {
                self.spawn_lane(&lane, None, false, Some(receipt.provider.clone()), None)?;
            }
            lane
        };
        let trail_session_id = format!(
            "session_hook_{}",
            crate::ids::short_hash(
                format!(
                    "{}:{}:{}",
                    receipt.workspace_id, receipt.provider, native_session_id
                )
                .as_bytes(),
                16,
            )
        );
        let session = self
            .start_lane_session(
                &lane,
                Some(format!("{} native session", receipt.provider)),
                Some(trail_session_id),
            )?
            .session;
        self.ensure_lane_agent_session(LaneAgentSessionInput {
            provider: receipt.provider.clone(),
            native_session_id: native_session_id.to_string(),
            parent_native_session_id: payload_string_value(
                payload,
                &["parent_session_id", "parentSessionId"],
            )
            .map(str::to_string),
            lane,
            trail_session_id: session.session_id,
            capture_run_id: capture_run.map(|capture_run| capture_run.capture_run_id),
            primary_transport: receipt.transport,
            transcript_identity: payload_string_value(
                payload,
                &["transcript_path", "transcriptPath"],
            )
            .map(str::to_string),
        })
    }

    /// Snapshot the real workspace used by a native provider into its virtual lane.
    /// Native hooks run in the user's checkout, not Trail's materialized lane workdir,
    /// so `record_lane_workdir_for_turn` cannot observe their writes. The baseline is
    /// captured before a turn begins; the terminal checkpoint is attached to that turn.
    fn record_native_agent_workspace_checkpoint(
        &mut self,
        lane: &str,
        session_id: &str,
        turn_id: Option<&str>,
        message: Option<String>,
    ) -> Result<LaneRecordReport> {
        validate_ref_segment(lane)?;
        let branch = self.lane_branch(lane)?;
        if let Some(turn_id) = turn_id {
            let turn = self.lane_turn(turn_id)?;
            if turn.lane_id != branch.lane_id || turn.session_id.as_deref() != Some(session_id) {
                return Err(Error::InvalidInput(format!(
                    "turn `{turn_id}` does not belong to native session `{session_id}`"
                )));
            }
            if turn.ended_at.is_some() {
                return Err(Error::InvalidInput(format!(
                    "turn `{turn_id}` is already ended"
                )));
            }
        }

        let head = self.get_ref(&branch.ref_name)?;
        self.refresh_worktree_index_streaming_report()?;
        let summaries = self.diff_root_to_worktree_index(&head.root_id)?;
        if summaries.is_empty() {
            self.set_worktree_index_baseline(&head.root_id)?;
            return Ok(LaneRecordReport {
                lane_id: branch.lane_id,
                operation: None,
                root_id: head.root_id,
                changed_paths: Vec::new(),
            });
        }

        self.ensure_lane_record_policy(&branch, &summaries)?;
        let paths = summaries
            .iter()
            .map(|summary| summary.path.clone())
            .collect::<Vec<_>>();
        let previous_files = self.load_root_files_for_paths(&head.root_id, &paths)?;
        let disk_files = self.scan_visible_files_for_paths(&paths)?;
        let actor = Actor::lane(lane);
        let change_id = self.allocate_change_id(&actor.id, "native_agent_checkpoint")?;
        let built = self.build_root_for_selected_record_incremental(
            &head.root_id,
            &previous_files,
            &disk_files,
            &paths,
            false,
            &change_id,
        )?;
        let diff = self.diff_file_maps(&previous_files, &built.files)?;
        if diff.changes.is_empty() {
            self.set_worktree_index_baseline(&head.root_id)?;
            return Ok(LaneRecordReport {
                lane_id: branch.lane_id,
                operation: None,
                root_id: head.root_id,
                changed_paths: Vec::new(),
            });
        }
        self.ensure_lane_record_file_size_policy(&built.files, &diff.summaries)?;

        let operation = Operation {
            version: OP_OBJECT_VERSION,
            change_id: change_id.clone(),
            kind: OperationKind::LaneRecord,
            parents: vec![head.change_id.clone()],
            before_root: Some(head.root_id.clone()),
            after_root: built.root_id.clone(),
            branch: branch.ref_name.clone(),
            actor,
            session_id: Some(session_id.to_string()),
            message: message.as_deref().map(redact_sensitive_text),
            changes: diff.changes,
            created_at: now_ts(),
        };
        let operation_id = self.store_operation(&operation)?;
        self.advance_ref_cas(&head, &change_id, &built.root_id, &operation_id)?;
        self.conn.execute(
            "UPDATE lane_branches SET head_change = ?1, head_root = ?2, updated_at = ?3 WHERE lane_id = ?4",
            params![change_id.0, built.root_id.0, now_ts(), branch.lane_id],
        )?;
        self.set_worktree_index_baseline(&built.root_id)?;

        let payload = serde_json::json!({
            "workspace": self.workspace_root,
            "root_id": built.root_id.0.clone(),
            "changed_paths": diff.summaries.iter().map(|item| item.path.clone()).collect::<Vec<_>>(),
            "checkpoint_kind": if turn_id.is_some() { "turn" } else { "baseline" }
        });
        if let Some(turn_id) = turn_id {
            self.add_lane_turn_event(
                turn_id,
                "workspace.checkpoint",
                Some(payload),
                Some(&change_id.0),
                None,
            )?;
            self.update_lane_turn_progress(
                turn_id,
                "workspace_checkpoint_recorded",
                Some(&change_id),
            )?;
        } else {
            self.add_lane_session_event(lane, session_id, "workspace.baseline", Some(payload))?;
        }
        Ok(LaneRecordReport {
            lane_id: branch.lane_id,
            operation: Some(change_id),
            root_id: built.root_id,
            changed_paths: diff.summaries,
        })
    }

    fn execute_agent_capture_actions(
        &mut self,
        lane: &str,
        mapping: &LaneAgentSession,
        event: &AgentLifecycleEvent,
        actions: &[AgentCaptureAction],
    ) -> Result<()> {
        for action in actions {
            match action {
                AgentCaptureAction::EnsureSession
                | AgentCaptureAction::CreateCaptureEpoch
                | AgentCaptureAction::WarnDuplicate
                | AgentCaptureAction::RecoverInterruptedTurn
                | AgentCaptureAction::DeferUntilFinalized => {}
                AgentCaptureAction::CaptureBaseline => {
                    self.record_native_agent_workspace_checkpoint(
                        lane,
                        &mapping.trail_session_id,
                        None,
                        Some(format!("agent {} turn baseline", event.provider)),
                    )?;
                }
                AgentCaptureAction::BeginTurn { synthetic } => {
                    if self
                        .open_turn_for_agent_session(&mapping.trail_session_id)?
                        .is_none()
                    {
                        let details = self.lane_details(lane)?;
                        let envelope = TurnEnvelope::new_agent_turn(TurnEnvelopeInput {
                            kind: "agent_hook_turn".to_string(),
                            protocol: "native-hooks".to_string(),
                            host: payload_string_value(
                                &event.payload,
                                &["host", "surface", "client"],
                            )
                            .map(str::to_string),
                            agent: Some(event.provider.clone()),
                            adapter: Some(format!("trail/{}@1", event.provider)),
                            provider: Some(event.provider.clone()),
                            model: payload_string_value(&event.payload, &["model"])
                                .map(str::to_string),
                            session: TurnEnvelopeSession {
                                trail_session_id: Some(mapping.trail_session_id.clone()),
                                upstream_session_id: event.native.session_id.clone(),
                                ..TurnEnvelopeSession::default()
                            },
                            prompt: TurnEnvelopePrompt {
                                summary: payload_string_value(
                                    &event.payload,
                                    &["prompt", "message"],
                                )
                                .map(|prompt| prompt.chars().take(160).collect()),
                                ..TurnEnvelopePrompt::default()
                            },
                            workspace: TurnEnvelopeWorkspace {
                                lane: Some(lane.to_string()),
                                cwd: payload_string_value(
                                    &event.payload,
                                    &["cwd", "workspaceRoot", "workspace_root"],
                                )
                                .map(str::to_string),
                                effective_cwd: details.branch.workdir.clone(),
                                workdir_mode: Some("native-provider".to_string()),
                                base_change: Some(details.branch.base_change.clone()),
                                before_change: Some(details.branch.head_change.clone()),
                            },
                        });
                        self.begin_lane_session_turn(
                            lane,
                            &mapping.trail_session_id,
                            Some(envelope.to_metadata_value()),
                        )?;
                        if *synthetic {
                            if let Some(turn_id) =
                                self.open_turn_for_agent_session(&mapping.trail_session_id)?
                            {
                                self.add_lane_turn_event(
                                    &turn_id,
                                    "diagnostic",
                                    Some(serde_json::json!({
                                        "code": "synthetic_turn_start",
                                        "native_turn_id": event.native.turn_id,
                                        "capture_run_id": event.capture_run_id,
                                    })),
                                    None,
                                    None,
                                )?;
                            }
                        }
                    }
                }
                AgentCaptureAction::AppendEvidence => {
                    let payload = serde_json::json!({
                        "schema": event.schema,
                        "version": event.version,
                        "event_id": event.event_id,
                        "native": event.native,
                        "correlation": event.correlation,
                        "evidence": event.evidence,
                        "payload": event.payload,
                    });
                    if let Some(turn_id) =
                        self.open_turn_for_agent_session(&mapping.trail_session_id)?
                    {
                        self.add_lane_turn_event(
                            &turn_id,
                            event.event_type.as_str(),
                            Some(payload),
                            None,
                            None,
                        )?;
                        self.capture_agent_event_message(&turn_id, event)?;
                    } else {
                        self.add_lane_session_event(
                            lane,
                            &mapping.trail_session_id,
                            event.event_type.as_str(),
                            Some(payload),
                        )?;
                    }
                }
                AgentCaptureAction::StartRootSpan => {
                    self.ensure_agent_trace_span(
                        mapping,
                        event,
                        "agent",
                        "Native agent turn",
                        None,
                        false,
                    )?;
                }
                AgentCaptureAction::StartToolSpan { synthetic } => {
                    self.ensure_agent_trace_span(
                        mapping,
                        event,
                        "tool",
                        payload_string_value(&event.payload, &["tool_name", "toolName", "tool"])
                            .unwrap_or("Native tool call"),
                        event.native.tool_id.as_deref(),
                        *synthetic,
                    )?;
                }
                AgentCaptureAction::EndToolSpan => {
                    self.end_agent_trace_span(
                        mapping,
                        event,
                        "tool",
                        event.native.tool_id.as_deref(),
                    )?;
                }
                AgentCaptureAction::StartSubagentSpan { synthetic } => {
                    self.ensure_agent_trace_span(
                        mapping,
                        event,
                        "subagent",
                        "Native subagent",
                        event.native.subagent_id.as_deref(),
                        *synthetic,
                    )?;
                }
                AgentCaptureAction::EndSubagentSpan => {
                    self.end_agent_trace_span(
                        mapping,
                        event,
                        "subagent",
                        event.native.subagent_id.as_deref(),
                    )?;
                }
                AgentCaptureAction::StartCompactionSpan => {
                    self.ensure_agent_trace_span(
                        mapping,
                        event,
                        "compaction",
                        "Native context compaction",
                        None,
                        false,
                    )?;
                }
                AgentCaptureAction::EndCompactionSpan => {
                    self.end_agent_trace_span(mapping, event, "compaction", None)?;
                }
                AgentCaptureAction::RequestTurnFinalization { outcome } => {
                    let lease = self.acquire_agent_finalization_lease(
                        &mapping.mapping_id,
                        &format!("receipt:{}", event.evidence.receipt_id),
                        30_000,
                        *outcome,
                        false,
                    )?;
                    if !lease.acquired {
                        return Err(Error::WorkspaceLocked(format!(
                            "agent session `{}` finalization is owned by `{}` until {}",
                            mapping.mapping_id,
                            lease
                                .mapping
                                .finalization_owner
                                .as_deref()
                                .unwrap_or("unknown"),
                            lease.expires_at
                        )));
                    }
                }
                AgentCaptureAction::ReconcileWorkdir => {
                    if let Some(turn_id) =
                        self.open_turn_for_agent_session(&mapping.trail_session_id)?
                    {
                        if self.lane_details(lane)?.branch.workdir.is_some() {
                            self.record_lane_workdir_for_turn(
                                lane,
                                &turn_id,
                                Some(format!("agent {} turn", event.provider)),
                            )?;
                        } else {
                            self.record_native_agent_workspace_checkpoint(
                                lane,
                                &mapping.trail_session_id,
                                Some(&turn_id),
                                Some(format!("agent {} turn checkpoint", event.provider)),
                            )?;
                        }
                    }
                }
                AgentCaptureAction::ImportTranscript => {
                    if let Err(error) = self.import_agent_transcript_snapshot(mapping, event) {
                        self.record_agent_capture_diagnostic(
                            lane,
                            mapping,
                            "TRANSCRIPT_IMPORT_DEFERRED",
                            &error.to_string(),
                        )?;
                    }
                }
                AgentCaptureAction::CloseTurn { outcome } => {
                    if let Some(turn_id) =
                        self.open_turn_for_agent_session(&mapping.trail_session_id)?
                    {
                        self.close_open_agent_trace_spans(
                            &turn_id,
                            agent_turn_outcome_name(*outcome),
                        )?;
                        self.end_lane_turn(&turn_id, agent_turn_outcome_name(*outcome))?;
                        self.create_turn_evidence_manifest(&turn_id)?;
                        self.classify_session_activity(&mapping.trail_session_id, 10_000)?;
                    }
                }
                AgentCaptureAction::FinalizeAttestation => {
                    if self
                        .lane_session_turns(&mapping.trail_session_id)?
                        .iter()
                        .any(|turn| turn.ended_at.is_some())
                    {
                        self.create_session_attestation(
                            &mapping.trail_session_id,
                            "native-hooks-on-finalization",
                            Some(serde_json::json!({
                                "provider": event.provider,
                                "receipt_id": event.evidence.receipt_id,
                            })),
                        )?;
                    }
                }
                AgentCaptureAction::RequestSessionClose => {
                    let _lock = self.acquire_write_lock()?;
                    self.conn.execute(
                        "UPDATE lane_agent_sessions SET session_close_requested = 1,
                                updated_at = ?2 WHERE mapping_id = ?1",
                        params![mapping.mapping_id, now_millis()],
                    )?;
                }
                AgentCaptureAction::CloseSession { interrupted } => {
                    let session = self.lane_session(&mapping.trail_session_id)?;
                    if session.status == "active" {
                        self.end_lane_session(
                            &mapping.trail_session_id,
                            if *interrupted {
                                "interrupted"
                            } else {
                                "completed"
                            },
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    fn ensure_agent_trace_span(
        &mut self,
        mapping: &LaneAgentSession,
        event: &AgentLifecycleEvent,
        span_type: &str,
        name: &str,
        native_id: Option<&str>,
        synthetic: bool,
    ) -> Result<()> {
        let Some(turn_id) = self.open_turn_for_agent_session(&mapping.trail_session_id)? else {
            return Ok(());
        };
        let spans = self.list_lane_trace_spans(
            None,
            Some(&mapping.trail_session_id),
            Some(&turn_id),
            None,
            1_000,
        )?;
        if spans.iter().any(|span| {
            span.attributes.as_ref().is_some_and(|attributes| {
                attributes
                    .get("lifecycle_event_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(event.event_id.as_str())
            })
        }) {
            return Ok(());
        }
        let parent_span_id = if span_type == "agent" {
            None
        } else {
            spans
                .iter()
                .find(|span| span.span_type == "agent" && span.ended_at.is_none())
                .map(|span| span.span_id.as_str())
        };
        self.start_lane_trace_span(
            &turn_id,
            span_type,
            name,
            parent_span_id,
            None,
            Some(serde_json::json!({
                "transport": "native-hooks",
                "provider": event.provider,
                "mapping_id": mapping.mapping_id,
                "lifecycle_event_id": event.event_id,
                "native_id": native_id,
                "synthetic": synthetic,
            })),
        )?;
        Ok(())
    }

    fn end_agent_trace_span(
        &mut self,
        mapping: &LaneAgentSession,
        event: &AgentLifecycleEvent,
        span_type: &str,
        native_id: Option<&str>,
    ) -> Result<()> {
        let Some(turn_id) = self.open_turn_for_agent_session(&mapping.trail_session_id)? else {
            return Ok(());
        };
        let spans = self.list_lane_trace_spans(
            None,
            Some(&mapping.trail_session_id),
            Some(&turn_id),
            None,
            1_000,
        )?;
        let span = spans.iter().find(|span| {
            span.span_type == span_type
                && span.ended_at.is_none()
                && (native_id.is_none()
                    || span.attributes.as_ref().is_some_and(|attributes| {
                        attributes
                            .get("native_id")
                            .and_then(serde_json::Value::as_str)
                            == native_id
                    }))
        });
        let Some(span) = span else {
            return Ok(());
        };
        let status = match event.event_type.kind() {
            AgentLifecycleEventKind::ToolFailed | AgentLifecycleEventKind::SubagentFailed => {
                "failed"
            }
            _ => "completed",
        };
        self.end_lane_trace_span(
            &span.span_id,
            status,
            Some(serde_json::json!({
                "lifecycle_event_id": event.event_id,
                "native_id": native_id,
            })),
        )?;
        Ok(())
    }

    fn close_open_agent_trace_spans(&mut self, turn_id: &str, status: &str) -> Result<()> {
        let mut spans = self.list_lane_trace_spans(None, None, Some(turn_id), None, 1_000)?;
        spans.retain(|span| span.ended_at.is_none());
        spans.sort_by(|left, right| {
            left.parent_span_id
                .is_none()
                .cmp(&right.parent_span_id.is_none())
                .then_with(|| right.started_at.cmp(&left.started_at))
        });
        for span in spans {
            self.end_lane_trace_span(
                &span.span_id,
                status,
                Some(serde_json::json!({"reason":"turn_finalization"})),
            )?;
        }
        Ok(())
    }

    fn import_agent_transcript_snapshot(
        &mut self,
        mapping: &LaneAgentSession,
        event: &AgentLifecycleEvent,
    ) -> Result<Option<LaneArtifact>> {
        const MAX_TRANSCRIPT_BYTES: u64 = 64 * 1024 * 1024;

        let canonical_export_locator = payload_string_value(
            &event.payload,
            &[
                "canonical_export_path",
                "canonicalExportPath",
                "export_path",
                "exportPath",
            ],
        )
        .map(str::to_string);
        let transcript_locator = payload_string_value(
            &event.payload,
            &[
                "transcript_path",
                "transcriptPath",
                "session_file",
                "sessionFile",
            ],
        )
        .map(str::to_string)
        .or_else(|| mapping.transcript_identity.clone());
        let (locator, artifact_kind, evidence_source, trust) =
            if let Some(locator) = canonical_export_locator {
                (
                    locator,
                    "export",
                    AgentEvidenceSource::CanonicalExport,
                    "provider-canonical-export",
                )
            } else if let Some(locator) = transcript_locator {
                (
                    locator,
                    "transcript",
                    AgentEvidenceSource::NativeTranscript,
                    "provider-native",
                )
            } else {
                return self.record_reconstructed_agent_transcript(mapping, event);
            };
        if locator.is_empty() || locator.chars().any(char::is_control) {
            return Err(Error::InvalidPath {
                path: "agent transcript locator".to_string(),
                reason: "locator is empty or contains control characters".to_string(),
            });
        }
        let raw = PathBuf::from(&locator);
        let candidate = if raw.is_absolute() {
            raw
        } else {
            let cwd =
                payload_string_value(&event.payload, &["cwd", "workspaceRoot", "workspace_root"])
                    .map(PathBuf::from)
                    .unwrap_or_else(|| self.workspace_root.clone());
            cwd.join(raw)
        };
        reject_agent_transcript_symlinks(&candidate)?;
        let canonical = candidate
            .canonicalize()
            .map_err(|error| Error::InvalidPath {
                path: "agent transcript locator".to_string(),
                reason: format!("cannot resolve provider transcript: {error}"),
            })?;
        if !self.agent_transcript_path_allowed(&event.provider, &canonical)? {
            return Err(Error::InvalidPath {
                path: "agent transcript locator".to_string(),
                reason: "provider transcript is outside the workspace and approved provider roots"
                    .to_string(),
            });
        }
        let metadata = std::fs::metadata(&canonical)?;
        if !metadata.is_file() {
            return Err(Error::InvalidPath {
                path: "agent transcript locator".to_string(),
                reason: "provider transcript is not a regular file".to_string(),
            });
        }
        if metadata.len() > MAX_TRANSCRIPT_BYTES {
            return Err(Error::InvalidInput(format!(
                "provider transcript is {} bytes; maximum is {MAX_TRANSCRIPT_BYTES}",
                metadata.len()
            )));
        }
        let content = std::fs::read(&canonical)?;
        if content.len() as u64 > MAX_TRANSCRIPT_BYTES {
            return Err(Error::InvalidInput(format!(
                "provider transcript grew beyond {MAX_TRANSCRIPT_BYTES} bytes while reading"
            )));
        }
        let prior_offset = mapping.transcript_offset.unwrap_or(0);
        let previous_offset = prior_offset.min(content.len() as u64);
        let end_offset = content.len() as u64;
        let content_digest = format!("sha256:{}", sha256_hex(&content));
        let previous_digest: Option<String> = self
            .conn
            .query_row(
                "SELECT content_digest FROM lane_artifacts
                 WHERE session_id = ?1 AND source = ?2
                 ORDER BY created_at DESC, artifact_id DESC LIMIT 1",
                params![
                    mapping.trail_session_id,
                    agent_evidence_source_name(evidence_source)
                ],
                |row| row.get(0),
            )
            .optional()?;
        let truncated = prior_offset > end_offset;
        let rewritten = previous_digest
            .as_deref()
            .is_some_and(|previous| previous != content_digest);
        let turn_id = self
            .open_turn_for_agent_session(&mapping.trail_session_id)?
            .or(self.agent_lifecycle_event_turn_id(&mapping.trail_session_id, &event.event_id)?);
        let lane = self.lane_name_by_id(&mapping.lane_id)?;
        let locator_digest = format!(
            "sha256:{}",
            sha256_hex(canonical.to_string_lossy().as_bytes())
        );
        let artifact = self.record_lane_artifact(LaneArtifactInput {
            lane,
            session_id: mapping.trail_session_id.clone(),
            turn_id,
            provider: event.provider.clone(),
            artifact_kind: artifact_kind.to_string(),
            format: agent_transcript_format(&canonical).to_string(),
            source: evidence_source,
            source_locator_redacted: canonical
                .file_name()
                .and_then(|name| name.to_str())
                .map(|name| format!("provider://sessions/{name}")),
            content,
            start_offset: Some(0),
            end_offset: Some(end_offset),
            redaction_profile: Some("trail-sensitive-json/v1".to_string()),
            trust: trust.to_string(),
            supersedes_artifact_id: None,
            metadata_json: Some(
                serde_json::json!({
                    "snapshot": "full",
                    "artifact_source": agent_evidence_source_name(evidence_source),
                    "delta_start_offset": previous_offset,
                    "delta_end_offset": end_offset,
                    "truncated": truncated,
                    "rewritten": rewritten,
                    "locator_digest": locator_digest,
                    "receipt_id": event.evidence.receipt_id,
                })
                .to_string(),
            ),
        })?;
        let _lock = self.acquire_write_lock()?;
        let changed = self.conn.execute(
            "UPDATE lane_agent_sessions
             SET transcript_identity = ?2, transcript_offset = ?3, updated_at = ?4
             WHERE mapping_id = ?1
               AND transcript_offset IS ?5",
            params![
                mapping.mapping_id,
                canonical.to_string_lossy(),
                i64::try_from(end_offset).unwrap_or(i64::MAX),
                now_millis(),
                mapping
                    .transcript_offset
                    .and_then(|offset| i64::try_from(offset).ok()),
            ],
        )?;
        if changed != 1 {
            return Err(Error::Conflict(format!(
                "agent transcript offset advanced concurrently for `{}`",
                mapping.mapping_id
            )));
        }
        Ok(Some(artifact))
    }

    fn record_reconstructed_agent_transcript(
        &mut self,
        mapping: &LaneAgentSession,
        event: &AgentLifecycleEvent,
    ) -> Result<Option<LaneArtifact>> {
        let turn_id = self
            .open_turn_for_agent_session(&mapping.trail_session_id)?
            .or(self.agent_lifecycle_event_turn_id(&mapping.trail_session_id, &event.event_id)?);
        let Some(turn_id) = turn_id else {
            return Ok(None);
        };
        let details = self.show_lane_turn(&turn_id)?;
        let content = serde_json::to_vec(&serde_json::json!({
            "schema": "trail.reconstructed_transcript",
            "version": 1,
            "provider": event.provider,
            "native_session_id": event.native.session_id,
            "native_turn_id": event.native.turn_id,
            "messages": details.messages,
            "events": details.events,
        }))?;
        let lane = self.lane_name_by_id(&mapping.lane_id)?;
        self.record_lane_artifact(LaneArtifactInput {
            lane,
            session_id: mapping.trail_session_id.clone(),
            turn_id: Some(turn_id),
            provider: event.provider.clone(),
            artifact_kind: "transcript".to_string(),
            format: "application/json".to_string(),
            source: AgentEvidenceSource::Reconstructed,
            source_locator_redacted: None,
            content,
            start_offset: None,
            end_offset: None,
            redaction_profile: Some("trail-sensitive-json/v1".to_string()),
            trust: "reconstructed-from-receipts".to_string(),
            supersedes_artifact_id: None,
            metadata_json: Some(
                serde_json::json!({
                    "receipt_id": event.evidence.receipt_id,
                    "fidelity": "reconstructed",
                })
                .to_string(),
            ),
        })
        .map(Some)
    }

    fn agent_transcript_path_allowed(&self, provider: &str, path: &Path) -> Result<bool> {
        if path.starts_with(&self.workspace_root) {
            return Ok(true);
        }
        let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
            return Ok(false);
        };
        let roots: &[&str] = match provider {
            "codex" => &[".codex/sessions"],
            "claude-code" => &[".claude/projects"],
            "pi" => &[".pi/agent/sessions"],
            "opencode" => &[".local/share/opencode", ".opencode"],
            "cursor" => &[".cursor/projects", ".cursor/sessions"],
            "gemini" => &[".gemini/tmp", ".gemini/sessions"],
            "copilot" => &[".copilot/session-state"],
            "grok" => &[".grok/sessions"],
            _ => &[],
        };
        for root in roots {
            let candidate = home.join(root);
            if candidate.exists() && path.starts_with(candidate.canonicalize()?) {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn record_agent_capture_diagnostic(
        &mut self,
        lane: &str,
        mapping: &LaneAgentSession,
        code: &str,
        message: &str,
    ) -> Result<()> {
        let payload = Some(serde_json::json!({
            "code": code,
            "message": redact_agent_capture_diagnostic(message),
            "mapping_id": mapping.mapping_id,
        }));
        if let Some(turn_id) = self.open_turn_for_agent_session(&mapping.trail_session_id)? {
            self.add_lane_turn_event(&turn_id, "diagnostic", payload, None, None)?;
        } else {
            self.add_lane_session_event(lane, &mapping.trail_session_id, "diagnostic", payload)?;
        }
        Ok(())
    }

    fn capture_agent_event_message(
        &mut self,
        turn_id: &str,
        event: &AgentLifecycleEvent,
    ) -> Result<()> {
        let (role, keys): (&str, &[&str]) = match event.event_type.kind() {
            AgentLifecycleEventKind::MessageUser => ("user", &["prompt", "message", "text"]),
            AgentLifecycleEventKind::MessageAssistantCompleted => (
                "assistant",
                &[
                    "last_assistant_message",
                    "lastAssistantMessage",
                    "assistant_response",
                    "assistantResponse",
                    "message",
                    "text",
                ],
            ),
            _ => return Ok(()),
        };
        if let Some(text) = payload_string_value(&event.payload, keys) {
            if !text.is_empty() {
                let duplicate = self
                    .show_lane_turn(turn_id)?
                    .messages
                    .iter()
                    .any(|message| message.role == role && message.body == text);
                if !duplicate {
                    self.add_lane_turn_message(turn_id, role, text)?;
                }
            }
        }
        Ok(())
    }

    fn match_acp_session_for_native_id(
        &self,
        provider: &str,
        native_session_id: &str,
    ) -> Result<Option<(String, String)>> {
        let mut statement = self.conn.prepare(
            "SELECT lane_id, trail_session_id FROM lane_acp_sessions
             WHERE (acp_session_id = ?1 OR upstream_session_id = ?1)
               AND (provider IS NULL OR provider = ?2)
             ORDER BY updated_at DESC LIMIT 2",
        )?;
        let rows = statement
            .query_map(params![native_session_id, provider], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        if rows.len() > 1 && rows[0] != rows[1] {
            return Err(Error::Conflict(format!(
                "native session `{native_session_id}` ambiguously matches multiple ACP sessions"
            )));
        }
        Ok(rows.into_iter().next())
    }

    fn hybrid_acp_owns_session(&self, trail_session_id: &str) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM lane_acp_sessions
                    WHERE trail_session_id = ?1
                      AND status IN ('starting', 'active', 'loaded', 'resumed')
                 )",
                params![trail_session_id],
                |row| row.get(0),
            )
            .map_err(Error::from)
    }

    fn update_agent_mapping_phase(
        &mut self,
        mapping_id: &str,
        expected_phase: AgentCapturePhase,
        expected_finalization_owner: Option<&str>,
        phase: AgentCapturePhase,
        pending_outcome: Option<AgentTurnOutcome>,
        session_close_requested: bool,
    ) -> Result<()> {
        let _lock = self.acquire_write_lock()?;
        let now = now_millis();
        let changed = self.conn.execute(
            "UPDATE lane_agent_sessions
             SET status = ?2, pending_turn_outcome = ?3,
                 session_close_requested = ?4,
                 finalization_owner = CASE WHEN ?2 = 'finalizing' THEN finalization_owner ELSE NULL END,
                 finalization_lease_expires_at = CASE WHEN ?2 = 'finalizing' THEN finalization_lease_expires_at ELSE NULL END,
                 updated_at = ?5
             WHERE mapping_id = ?1 AND status = ?6
               AND (?6 != 'finalizing' OR
                    (finalization_owner = ?7 AND finalization_lease_expires_at > ?5))",
            params![
                mapping_id,
                agent_capture_phase_name(phase),
                pending_outcome.map(agent_turn_outcome_name),
                if session_close_requested { 1 } else { 0 },
                now,
                agent_capture_phase_name(expected_phase),
                expected_finalization_owner,
            ],
        )?;
        if changed != 1 {
            return Err(Error::WorkspaceLocked(format!(
                "agent session `{mapping_id}` changed phase or lost its finalization lease"
            )));
        }
        Ok(())
    }

    fn lane_name_by_id(&self, lane_id: &str) -> Result<String> {
        self.conn
            .query_row(
                "SELECT name FROM lanes WHERE lane_id = ?1",
                params![lane_id],
                |row| row.get(0),
            )
            .optional()?
            .ok_or_else(|| Error::InvalidInput(format!("lane id `{lane_id}` not found")))
    }

    fn open_turn_for_agent_session(&self, session_id: &str) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT turn_id FROM lane_turns
                 WHERE session_id = ?1 AND ended_at IS NULL
                 ORDER BY started_at DESC, rowid DESC LIMIT 1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::from)
    }

    fn agent_lifecycle_event_already_recorded(
        &self,
        session_id: &str,
        event_id: &str,
    ) -> Result<bool> {
        self.conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM lane_events
                    WHERE session_id = ?1 AND json_extract(payload_json, '$.event_id') = ?2
                 )",
                params![session_id, event_id],
                |row| row.get::<_, bool>(0),
            )
            .map_err(Error::from)
    }

    fn agent_lifecycle_event_turn_id(
        &self,
        session_id: &str,
        event_id: &str,
    ) -> Result<Option<String>> {
        self.conn
            .query_row(
                "SELECT turn_id FROM lane_events
                 WHERE session_id = ?1 AND json_extract(payload_json, '$.event_id') = ?2
                   AND turn_id IS NOT NULL
                 ORDER BY created_at DESC, rowid DESC LIMIT 1",
                params![session_id, event_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(Error::from)
    }

    /// Attach a durable receipt to a resolved native session and allocate its logical order.
    pub fn assign_agent_hook_receipt_mapping(
        &mut self,
        receipt_id: &str,
        mapping_id: &str,
    ) -> Result<AgentHookReceipt> {
        let _lock = self.acquire_write_lock()?;
        validate_agent_capture_id("receipt id", receipt_id, 256)?;
        validate_agent_capture_id("agent session mapping id", mapping_id, 256)?;
        let receipt = self.agent_hook_receipt(receipt_id)?;
        if let Some(existing_mapping) = receipt.mapping_id.as_deref() {
            if existing_mapping == mapping_id {
                return Ok(receipt);
            }
            return Err(Error::InvalidInput(format!(
                "agent hook receipt `{receipt_id}` already belongs to mapping `{existing_mapping}`"
            )));
        }

        let mapping: Option<(String, String, i64)> = self
            .conn
            .query_row(
                "SELECT workspace_id, provider, next_receive_sequence
                 FROM lane_agent_sessions WHERE mapping_id = ?1",
                params![mapping_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((workspace_id, provider, sequence)) = mapping else {
            return Err(Error::InvalidInput(format!(
                "agent session mapping `{mapping_id}` not found"
            )));
        };
        if workspace_id != receipt.workspace_id || provider != receipt.provider {
            return Err(Error::InvalidInput(format!(
                "agent hook receipt `{receipt_id}` and mapping `{mapping_id}` have different workspace or provider identities"
            )));
        }

        self.conn
            .execute_batch("SAVEPOINT assign_agent_receipt_mapping")?;
        let update = (|| -> Result<()> {
            let mapping_updated = self.conn.execute(
                "UPDATE lane_agent_sessions
                 SET next_receive_sequence = next_receive_sequence + 1, updated_at = ?1
                 WHERE mapping_id = ?2 AND next_receive_sequence = ?3",
                params![now_millis(), mapping_id, sequence],
            )?;
            if mapping_updated != 1 {
                return Err(Error::WorkspaceLocked(format!(
                    "agent session mapping `{mapping_id}` receive sequence changed concurrently"
                )));
            }
            self.conn.execute(
                "UPDATE agent_hook_receipts
                 SET mapping_id = ?1, receive_sequence = ?2, updated_at = ?3
                 WHERE receipt_id = ?4 AND mapping_id IS NULL",
                params![mapping_id, sequence, now_millis(), receipt_id],
            )?;
            Ok(())
        })();
        match update {
            Ok(()) => self
                .conn
                .execute_batch("RELEASE SAVEPOINT assign_agent_receipt_mapping")?,
            Err(error) => {
                let _ = self.conn.execute_batch(
                    "ROLLBACK TO SAVEPOINT assign_agent_receipt_mapping;
                     RELEASE SAVEPOINT assign_agent_receipt_mapping",
                );
                return Err(error);
            }
        }
        self.agent_hook_receipt(receipt_id)
    }

    fn try_agent_hook_receipt_by_dedupe_key(
        &self,
        workspace_id: &str,
        provider: &str,
        dedupe_key: &str,
    ) -> Result<Option<AgentHookReceipt>> {
        self.conn
            .query_row(
                "SELECT receipt_id, workspace_id, installation_id, mapping_id, provider,
                        native_event, native_session_id, native_turn_id, transport, dedupe_key,
                        payload_digest, raw_object_id, raw_artifact_id, receive_sequence, status,
                        attempt_count, next_attempt_at, diagnostic, occurred_at, received_at,
                        processed_at, updated_at
                 FROM agent_hook_receipts
                 WHERE workspace_id = ?1 AND provider = ?2 AND dedupe_key = ?3",
                params![workspace_id, provider, dedupe_key],
                agent_hook_receipt_row,
            )
            .optional()
            .map_err(Error::from)
    }
}

const AGENT_HOOK_INSTALLATION_BY_ID_SELECT: &str =
    "SELECT installation_id, workspace_id, provider, scope, config_path,
            lane_id, manifest_digest, ownership_inventory_json,
            config_before_digest, config_after_digest, adapter_version,
            provider_version_range, detected_provider_version, capability_status,
            status, installed_at, verified_at, last_receipt_at
     FROM agent_hook_installations
     WHERE installation_id = ?1 AND workspace_id = ?2";

const LANE_ARTIFACT_BY_ID_SELECT: &str =
    "SELECT artifact_id, workspace_id, lane_id, session_id, turn_id, provider,
            artifact_kind, format, source, source_locator_redacted, content_object_id,
            content_digest, size_bytes, start_offset, end_offset, redaction_profile,
            retention_status, trust, supersedes_artifact_id, created_at, metadata_json
     FROM lane_artifacts WHERE artifact_id = ?1";

fn map_lane_artifact(row: &rusqlite::Row<'_>) -> rusqlite::Result<LaneArtifact> {
    let source: String = row.get(8)?;
    let source = parse_agent_evidence_source(&source).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(8, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(LaneArtifact {
        artifact_id: row.get(0)?,
        workspace_id: row.get(1)?,
        lane_id: row.get(2)?,
        session_id: row.get(3)?,
        turn_id: row.get(4)?,
        provider: row.get(5)?,
        artifact_kind: row.get(6)?,
        format: row.get(7)?,
        source,
        source_locator_redacted: row.get(9)?,
        content_object_id: row.get::<_, Option<String>>(10)?.map(ObjectId),
        content_digest: row.get(11)?,
        size_bytes: u64::try_from(row.get::<_, i64>(12)?).unwrap_or(0),
        start_offset: row
            .get::<_, Option<i64>>(13)?
            .and_then(|value| u64::try_from(value).ok()),
        end_offset: row
            .get::<_, Option<i64>>(14)?
            .and_then(|value| u64::try_from(value).ok()),
        redaction_profile: row.get(15)?,
        retention_status: row.get(16)?,
        trust: row.get(17)?,
        supersedes_artifact_id: row.get(18)?,
        created_at: row.get(19)?,
        metadata_json: row.get(20)?,
    })
}

const AGENT_HOOK_INSTALLATION_LIST_SELECT: &str =
    "SELECT installation_id, workspace_id, provider, scope, config_path,
            lane_id, manifest_digest, ownership_inventory_json,
            config_before_digest, config_after_digest, adapter_version,
            provider_version_range, detected_provider_version, capability_status,
            status, installed_at, verified_at, last_receipt_at
     FROM agent_hook_installations
     WHERE workspace_id = ?1
     ORDER BY provider, scope, config_path";

const AGENT_HOOK_INSTALLATION_BY_PROVIDER_SELECT: &str =
    "SELECT installation_id, workspace_id, provider, scope, config_path,
            lane_id, manifest_digest, ownership_inventory_json,
            config_before_digest, config_after_digest, adapter_version,
            provider_version_range, detected_provider_version, capability_status,
            status, installed_at, verified_at, last_receipt_at
     FROM agent_hook_installations
     WHERE workspace_id = ?1 AND provider = ?2
     ORDER BY scope, config_path";

fn map_agent_hook_installation(
    row: &rusqlite::Row<'_>,
) -> rusqlite::Result<AgentHookInstallationRecord> {
    let scope: String = row.get(3)?;
    let scope = match scope.as_str() {
        "project" => AgentHookInstallScope::Project,
        "user" => AgentHookInstallScope::User,
        other => {
            return Err(rusqlite::Error::FromSqlConversionFailure(
                3,
                rusqlite::types::Type::Text,
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unknown agent hook installation scope `{other}`"),
                )
                .into(),
            ))
        }
    };
    let inventory: String = row.get(7)?;
    let ownership_inventory = serde_json::from_str(&inventory).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(7, rusqlite::types::Type::Text, Box::new(error))
    })?;
    Ok(AgentHookInstallationRecord {
        installation_id: row.get(0)?,
        workspace_id: row.get(1)?,
        provider: row.get(2)?,
        scope,
        config_path: PathBuf::from(row.get::<_, String>(4)?),
        lane_id: row.get(5)?,
        manifest_digest: row.get(6)?,
        ownership_inventory,
        config_before_digest: row.get(8)?,
        config_after_digest: row.get(9)?,
        adapter_version: row.get(10)?,
        provider_version_range: row.get(11)?,
        detected_provider_version: row.get(12)?,
        capability_status: row.get(13)?,
        status: row.get(14)?,
        installed_at: row.get(15)?,
        verified_at: row.get(16)?,
        last_receipt_at: row.get(17)?,
    })
}

const AGENT_CAPTURE_RUN_SELECT_BY_ID: &str =
    "SELECT capture_run_id, workspace_id, lane_id, workdir, canonical_workdir,
            owner_agent, owner_session_id, executor_agent, work_item_id, status,
            created_at, updated_at, expires_at, ended_at, metadata_json
     FROM agent_capture_runs WHERE capture_run_id = ?1";

fn agent_capture_run_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentCaptureRun> {
    Ok(AgentCaptureRun {
        capture_run_id: row.get(0)?,
        workspace_id: row.get(1)?,
        lane_id: row.get(2)?,
        workdir: row.get(3)?,
        canonical_workdir: row.get(4)?,
        owner_agent: row.get(5)?,
        owner_session_id: row.get(6)?,
        executor_agent: row.get(7)?,
        work_item_id: row.get(8)?,
        status: row.get(9)?,
        created_at: row.get(10)?,
        updated_at: row.get(11)?,
        expires_at: row.get(12)?,
        ended_at: row.get(13)?,
        metadata_json: row.get(14)?,
    })
}

const LANE_AGENT_SESSION_SELECT_BY_ID: &str =
    "SELECT mapping_id, workspace_id, provider, native_session_id,
            parent_native_session_id, trail_session_id, lane_id, capture_run_id,
            primary_transport, transcript_identity, transcript_offset, resume_json,
            last_attestation_id, status, pending_turn_outcome,
            session_close_requested, capture_epoch, finalization_owner,
            finalization_lease_expires_at, next_receive_sequence, created_at, updated_at
     FROM lane_agent_sessions WHERE mapping_id = ?1";

fn lane_agent_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<LaneAgentSession> {
    let primary_transport = parse_agent_capture_transport(row.get::<_, String>(8)?.as_str())
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    let status =
        parse_agent_capture_phase(row.get::<_, String>(13)?.as_str()).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                13,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    let pending_turn_outcome = row
        .get::<_, Option<String>>(14)?
        .map(|value| parse_agent_turn_outcome(&value))
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                14,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    Ok(LaneAgentSession {
        mapping_id: row.get(0)?,
        workspace_id: row.get(1)?,
        provider: row.get(2)?,
        native_session_id: row.get(3)?,
        parent_native_session_id: row.get(4)?,
        trail_session_id: row.get(5)?,
        lane_id: row.get(6)?,
        capture_run_id: row.get(7)?,
        primary_transport,
        transcript_identity: row.get(9)?,
        transcript_offset: row
            .get::<_, Option<i64>>(10)?
            .map(u64::try_from)
            .transpose()
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    10,
                    rusqlite::types::Type::Integer,
                    Box::new(error),
                )
            })?,
        resume_json: row.get(11)?,
        last_attestation_id: row.get(12)?,
        status,
        pending_turn_outcome,
        session_close_requested: row.get(15)?,
        capture_epoch: u64::try_from(row.get::<_, i64>(16)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                16,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        finalization_owner: row.get(17)?,
        finalization_lease_expires_at: row.get(18)?,
        next_receive_sequence: u64::try_from(row.get::<_, i64>(19)?).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                19,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?,
        created_at: row.get(20)?,
        updated_at: row.get(21)?,
    })
}

fn agent_hook_receipt_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AgentHookReceipt> {
    let transport =
        parse_agent_capture_transport(row.get::<_, String>(8)?.as_str()).map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                8,
                rusqlite::types::Type::Text,
                Box::new(error),
            )
        })?;
    let receive_sequence = row
        .get::<_, Option<i64>>(13)?
        .map(u64::try_from)
        .transpose()
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(
                13,
                rusqlite::types::Type::Integer,
                Box::new(error),
            )
        })?;
    let attempt_count = u32::try_from(row.get::<_, i64>(15)?).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            15,
            rusqlite::types::Type::Integer,
            Box::new(error),
        )
    })?;
    Ok(AgentHookReceipt {
        receipt_id: row.get(0)?,
        workspace_id: row.get(1)?,
        installation_id: row.get(2)?,
        mapping_id: row.get(3)?,
        provider: row.get(4)?,
        native_event: row.get(5)?,
        native_session_id: row.get(6)?,
        native_turn_id: row.get(7)?,
        transport,
        dedupe_key: row.get(9)?,
        payload_digest: row.get(10)?,
        raw_object_id: ObjectId(row.get(11)?),
        raw_artifact_id: row.get(12)?,
        receive_sequence,
        status: row.get(14)?,
        attempt_count,
        next_attempt_at: row.get(16)?,
        diagnostic: row.get(17)?,
        occurred_at: row.get(18)?,
        received_at: row.get(19)?,
        processed_at: row.get(20)?,
        updated_at: row.get(21)?,
    })
}

fn validate_agent_receipt_input(input: &AgentHookReceiptInput) -> Result<()> {
    validate_agent_provider(&input.provider)?;
    validate_agent_capture_id("native event", &input.native_event, 256)?;
    validate_agent_capture_id("dedupe key", &input.dedupe_key, 1024)?;
    if let Some(value) = input.installation_id.as_deref() {
        validate_agent_capture_id("installation id", value, 256)?;
    }
    if let Some(value) = input.native_session_id.as_deref() {
        validate_agent_capture_id("native session id", value, 1024)?;
    }
    if let Some(value) = input.native_turn_id.as_deref() {
        validate_agent_capture_id("native turn id", value, 1024)?;
    }
    if input.occurred_at.is_some_and(|value| value < 0) {
        return Err(Error::InvalidInput(
            "agent hook occurred_at must be non-negative milliseconds".to_string(),
        ));
    }
    Ok(())
}

fn validate_agent_provider(value: &str) -> Result<()> {
    validate_agent_capture_id("provider", value, 64)?;
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(Error::InvalidInput(
            "agent provider must use lowercase ASCII letters, digits, and hyphens".to_string(),
        ));
    }
    Ok(())
}

fn validate_agent_capture_id(label: &str, value: &str, maximum: usize) -> Result<()> {
    if value.is_empty() || value.len() > maximum || value.chars().any(char::is_control) {
        return Err(Error::InvalidInput(format!(
            "agent {label} must contain 1 to {maximum} non-control bytes"
        )));
    }
    Ok(())
}

fn validate_agent_receipt_status(status: &str) -> Result<()> {
    match status {
        "received" | "processing" | "processed" | "retry" | "quarantined" | "discarded" => Ok(()),
        _ => Err(Error::InvalidInput(format!(
            "unknown agent hook receipt status `{status}`"
        ))),
    }
}

fn validate_agent_capture_lease_ms(lease_ms: u64) -> Result<()> {
    if !(1_000..=86_400_000).contains(&lease_ms) {
        return Err(Error::InvalidInput(
            "agent capture run lease must be between 1000 and 86400000 milliseconds".to_string(),
        ));
    }
    Ok(())
}

fn agent_capture_transport_name(transport: AgentCaptureTransport) -> &'static str {
    match transport {
        AgentCaptureTransport::Acp => "acp",
        AgentCaptureTransport::NativeHooks => "native-hooks",
        AgentCaptureTransport::Terminal => "terminal",
        AgentCaptureTransport::Hybrid => "hybrid",
    }
}

fn agent_evidence_source_name(source: AgentEvidenceSource) -> &'static str {
    match source {
        AgentEvidenceSource::Acp => "acp",
        AgentEvidenceSource::NativeHook => "native_hook",
        AgentEvidenceSource::NativeTranscript => "native_transcript",
        AgentEvidenceSource::CanonicalExport => "canonical_export",
        AgentEvidenceSource::WorkdirObserved => "workdir_observed",
        AgentEvidenceSource::Reconstructed => "reconstructed",
    }
}

fn parse_agent_evidence_source(value: &str) -> Result<AgentEvidenceSource> {
    match value {
        "acp" => Ok(AgentEvidenceSource::Acp),
        "native_hook" => Ok(AgentEvidenceSource::NativeHook),
        "native_transcript" => Ok(AgentEvidenceSource::NativeTranscript),
        "canonical_export" => Ok(AgentEvidenceSource::CanonicalExport),
        "workdir_observed" => Ok(AgentEvidenceSource::WorkdirObserved),
        "reconstructed" => Ok(AgentEvidenceSource::Reconstructed),
        _ => Err(Error::Corrupt(format!(
            "unknown agent evidence source `{value}`"
        ))),
    }
}

fn reject_agent_transcript_symlinks(path: &Path) -> Result<()> {
    if std::fs::symlink_metadata(path)
        .map(|metadata| metadata.file_type().is_symlink())
        .unwrap_or(false)
    {
        return Err(Error::InvalidPath {
            path: "agent transcript locator".to_string(),
            reason: "provider transcript locator is a symbolic link".to_string(),
        });
    }
    Ok(())
}

fn agent_transcript_format(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("jsonl" | "ndjson") => "application/x-ndjson",
        Some("json") => "application/json",
        Some("md" | "markdown") => "text/markdown",
        Some("txt") => "text/plain",
        _ => "application/octet-stream",
    }
}

fn parse_agent_capture_transport(value: &str) -> std::result::Result<AgentCaptureTransport, Error> {
    match value {
        "acp" => Ok(AgentCaptureTransport::Acp),
        "native-hooks" => Ok(AgentCaptureTransport::NativeHooks),
        "terminal" => Ok(AgentCaptureTransport::Terminal),
        "hybrid" => Ok(AgentCaptureTransport::Hybrid),
        _ => Err(Error::Corrupt(format!(
            "unknown agent capture transport `{value}`"
        ))),
    }
}

fn payload_string_value<'a>(payload: &'a serde_json::Value, keys: &[&str]) -> Option<&'a str> {
    keys.iter().find_map(|key| {
        payload
            .get(*key)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty())
    })
}

fn redact_agent_capture_diagnostic(value: &str) -> String {
    redact_sensitive_text(value).chars().take(2_048).collect()
}

fn agent_capture_phase_name(value: AgentCapturePhase) -> &'static str {
    match value {
        AgentCapturePhase::Idle => "idle",
        AgentCapturePhase::Active => "active",
        AgentCapturePhase::Finalizing => "finalizing",
        AgentCapturePhase::Ended => "ended",
        AgentCapturePhase::Interrupted => "interrupted",
    }
}

fn parse_agent_capture_phase(value: &str) -> std::result::Result<AgentCapturePhase, Error> {
    match value {
        "idle" => Ok(AgentCapturePhase::Idle),
        "active" => Ok(AgentCapturePhase::Active),
        "finalizing" => Ok(AgentCapturePhase::Finalizing),
        "ended" => Ok(AgentCapturePhase::Ended),
        "interrupted" => Ok(AgentCapturePhase::Interrupted),
        _ => Err(Error::Corrupt(format!(
            "unknown agent capture phase `{value}`"
        ))),
    }
}

fn agent_turn_outcome_name(value: AgentTurnOutcome) -> &'static str {
    match value {
        AgentTurnOutcome::Completed => "completed",
        AgentTurnOutcome::Failed => "failed",
        AgentTurnOutcome::Cancelled => "cancelled",
        AgentTurnOutcome::Interrupted => "interrupted",
    }
}

fn parse_agent_turn_outcome(value: &str) -> std::result::Result<AgentTurnOutcome, Error> {
    match value {
        "completed" => Ok(AgentTurnOutcome::Completed),
        "failed" => Ok(AgentTurnOutcome::Failed),
        "cancelled" => Ok(AgentTurnOutcome::Cancelled),
        "interrupted" => Ok(AgentTurnOutcome::Interrupted),
        _ => Err(Error::Corrupt(format!(
            "unknown agent turn outcome `{value}`"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn receipt_ingress_is_redacted_durable_and_idempotent() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let input = AgentHookReceiptInput {
            installation_id: None,
            provider: "codex".to_string(),
            native_event: "AfterTool".to_string(),
            native_session_id: Some("native-1".to_string()),
            native_turn_id: Some("turn-1".to_string()),
            transport: AgentCaptureTransport::NativeHooks,
            dedupe_key: "native-1:AfterTool:1".to_string(),
            payload: serde_json::json!({
                "tool": "shell",
                "api_key": "must-not-survive",
                "output": "authorization: Bearer must-not-survive"
            }),
            occurred_at: Some(1),
        };
        let first = db.persist_agent_hook_receipt(input.clone()).unwrap();
        assert!(!first.duplicate);
        let second = db.persist_agent_hook_receipt(input).unwrap();
        assert!(second.duplicate);
        assert_eq!(first.receipt.receipt_id, second.receipt.receipt_id);

        let stored: AgentHookReceiptObject = db
            .get_object(AGENT_HOOK_RECEIPT_OBJECT_KIND, &first.receipt.raw_object_id)
            .unwrap();
        let rendered = serde_json::to_string(&stored.payload).unwrap();
        assert!(!rendered.contains("must-not-survive"));
        assert!(rendered.contains("REDACTED"));

        drop(db);
        let reopened = Trail::open(temp.path()).unwrap();
        assert_eq!(
            reopened
                .agent_hook_receipt(&first.receipt.receipt_id)
                .unwrap(),
            first.receipt
        );
    }

    #[test]
    fn dedupe_key_collision_with_different_payload_fails_closed() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let mut input = AgentHookReceiptInput {
            installation_id: None,
            provider: "claude-code".to_string(),
            native_event: "Stop".to_string(),
            native_session_id: Some("native-1".to_string()),
            native_turn_id: None,
            transport: AgentCaptureTransport::NativeHooks,
            dedupe_key: "native-1:Stop:1".to_string(),
            payload: serde_json::json!({"result": 1}),
            occurred_at: None,
        };
        db.persist_agent_hook_receipt(input.clone()).unwrap();
        input.payload = serde_json::json!({"result": 2});
        assert!(db.persist_agent_hook_receipt(input).is_err());
    }

    #[test]
    fn receipt_ingress_rejects_payloads_over_the_hard_limit() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let result = db.persist_agent_hook_receipt(AgentHookReceiptInput {
            installation_id: None,
            provider: "codex".to_string(),
            native_event: "AfterTool".to_string(),
            native_session_id: Some("oversized-session".to_string()),
            native_turn_id: Some("oversized-turn".to_string()),
            transport: AgentCaptureTransport::NativeHooks,
            dedupe_key: "oversized:one".to_string(),
            payload: serde_json::json!({
                "output": "x".repeat(AGENT_LIFECYCLE_MAX_PAYLOAD_BYTES + 1)
            }),
            occurred_at: None,
        });
        assert!(result.is_err());
        assert!(db
            .list_agent_hook_receipts(Some("codex"), None, 10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn durable_receipts_replay_into_one_native_session_turn_and_message() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        let transcript_path = temp.path().join("claude-session.jsonl");
        let transcript = b"{\"type\":\"user\",\"message\":\"make the change\"}\n{\"type\":\"assistant\",\"message\":\"done\"}\n";
        std::fs::write(&transcript_path, transcript).unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let persist = |db: &mut Trail, event: &str, dedupe: &str, payload: serde_json::Value| {
            db.persist_agent_hook_receipt(AgentHookReceiptInput {
                installation_id: None,
                provider: "claude-code".to_string(),
                native_event: event.to_string(),
                native_session_id: Some("native-replay-1".to_string()),
                native_turn_id: payload
                    .get("turn_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
                transport: AgentCaptureTransport::NativeHooks,
                dedupe_key: dedupe.to_string(),
                payload,
                occurred_at: None,
            })
            .unwrap()
            .receipt
            .receipt_id
        };
        let start = persist(
            &mut db,
            "SessionStart",
            "replay:start",
            serde_json::json!({
                "session_id":"native-replay-1",
                "source":"startup",
                "transcript_path": transcript_path,
            }),
        );
        let prompt = persist(
            &mut db,
            "UserPromptSubmit",
            "replay:prompt",
            serde_json::json!({
                "session_id":"native-replay-1",
                "turn_id":"native-turn-1",
                "prompt":"make the change"
            }),
        );
        let tool_start = persist(
            &mut db,
            "PreToolUse",
            "replay:tool:start",
            serde_json::json!({
                "session_id":"native-replay-1",
                "turn_id":"native-turn-1",
                "tool_use_id":"tool-1",
                "tool_name":"shell"
            }),
        );
        let tool_end = persist(
            &mut db,
            "PostToolUse",
            "replay:tool:end",
            serde_json::json!({
                "session_id":"native-replay-1",
                "turn_id":"native-turn-1",
                "tool_use_id":"tool-1",
                "result":"ok"
            }),
        );
        let stop = persist(
            &mut db,
            "Stop",
            "replay:stop",
            serde_json::json!({
                "session_id":"native-replay-1",
                "turn_id":"native-turn-1",
                "last_assistant_message":"done"
            }),
        );
        let late_subagent_stop = persist(
            &mut db,
            "SubagentStop",
            "replay:subagent:late-stop",
            serde_json::json!({
                "session_id":"native-replay-1",
                "turn_id":"native-turn-1",
                "agent_id":"subagent-1",
                "status":"completed"
            }),
        );
        db.replay_agent_hook_receipt(&start).unwrap();
        let prompt_report = db.replay_agent_hook_receipt(&prompt).unwrap();
        assert_eq!(prompt_report.normalized_events.len(), 2);
        db.replay_agent_hook_receipt(&tool_start).unwrap();
        db.replay_agent_hook_receipt(&tool_end).unwrap();
        let stop_report = db.replay_agent_hook_receipt(&stop).unwrap();
        assert_eq!(stop_report.receipt.status, "processed");
        let mapping = stop_report.mapping.unwrap();
        assert_eq!(mapping.status, AgentCapturePhase::Idle);
        let late_report = db.replay_agent_hook_receipt(&late_subagent_stop).unwrap();
        assert_eq!(late_report.mapping.unwrap().status, AgentCapturePhase::Idle);
        let turns = db.lane_session_turns(&mapping.trail_session_id).unwrap();
        assert_eq!(turns.len(), 1);
        assert_eq!(turns[0].status, "completed");
        let details = db.show_lane_turn(&turns[0].turn_id).unwrap();
        assert_eq!(details.messages.len(), 1);
        assert_eq!(details.messages[0].body, "make the change");
        let spans = db
            .list_lane_trace_spans(
                None,
                Some(&mapping.trail_session_id),
                Some(&turns[0].turn_id),
                None,
                10,
            )
            .unwrap();
        assert_eq!(spans.len(), 2);
        assert!(spans.iter().all(|span| span.ended_at.is_some()));
        assert!(spans.iter().any(|span| span.span_type == "agent"));
        assert!(spans.iter().any(|span| span.span_type == "tool"));
        let artifacts = db
            .list_lane_artifacts(&mapping.trail_session_id, Some(&turns[0].turn_id), 10)
            .unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact_kind, "transcript");
        assert_eq!(artifacts[0].start_offset, Some(0));
        assert_eq!(artifacts[0].end_offset, Some(transcript.len() as u64));
        assert_eq!(
            db.lane_artifact_content(&artifacts[0].artifact_id).unwrap(),
            transcript
        );
        let mapping_after_import = db.lane_agent_session(&mapping.mapping_id).unwrap();
        assert_eq!(
            mapping_after_import.transcript_offset,
            Some(transcript.len() as u64)
        );
        assert_eq!(
            db.turn_evidence_manifest(&turns[0].turn_id)
                .unwrap()
                .turn_id,
            turns[0].turn_id
        );

        let end = persist(
            &mut db,
            "SessionEnd",
            "replay:end",
            serde_json::json!({"session_id":"native-replay-1","reason":"exit"}),
        );
        let end_report = db.replay_agent_hook_receipt(&end).unwrap();
        assert_eq!(end_report.mapping.unwrap().status, AgentCapturePhase::Ended);
        let attestations = db
            .list_session_attestations(&mapping.trail_session_id)
            .unwrap();
        assert_eq!(attestations.len(), 1);
        assert!(
            db.verify_session_attestation(&attestations[0].attestation_id)
                .unwrap()
                .valid
        );
    }

    #[test]
    fn native_replay_checkpoints_real_workspace_changes_and_resolves_native_id() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "before\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let persist = |db: &mut Trail, event: &str, suffix: &str, payload: serde_json::Value| {
            db.persist_agent_hook_receipt(AgentHookReceiptInput {
                installation_id: None,
                provider: "claude-code".to_string(),
                native_event: event.to_string(),
                native_session_id: Some("native-workspace-change".to_string()),
                native_turn_id: None,
                transport: AgentCaptureTransport::NativeHooks,
                dedupe_key: format!("native-workspace-change:{suffix}"),
                payload,
                occurred_at: None,
            })
            .unwrap()
            .receipt
            .receipt_id
        };
        let started = persist(
            &mut db,
            "SessionStart",
            "start",
            serde_json::json!({"session_id":"native-workspace-change","cwd":temp.path().to_string_lossy()}),
        );
        let prompt = persist(
            &mut db,
            "UserPromptSubmit",
            "prompt",
            serde_json::json!({"session_id":"native-workspace-change","prompt":"change README","cwd":temp.path().to_string_lossy()}),
        );
        db.replay_agent_hook_receipt(&started).unwrap();
        let mapping = db
            .replay_agent_hook_receipt(&prompt)
            .unwrap()
            .mapping
            .unwrap();
        assert_eq!(
            db.try_lane_agent_session_by_native_id("native-workspace-change")
                .unwrap()
                .unwrap()
                .mapping_id,
            mapping.mapping_id
        );

        std::fs::write(temp.path().join("README.md"), "after\n").unwrap();
        let stop = persist(
            &mut db,
            "Stop",
            "stop",
            serde_json::json!({"session_id":"native-workspace-change","cwd":temp.path().to_string_lossy()}),
        );
        let mapping = db
            .replay_agent_hook_receipt(&stop)
            .unwrap()
            .mapping
            .unwrap();
        let lane = db.lane_name_by_id(&mapping.lane_id).unwrap();
        let branch = db.lane_branch(&lane).unwrap();
        let diff = db
            .diff_refs(&branch.base_change.0, &branch.head_change.0, false)
            .unwrap();
        assert_eq!(diff.files.len(), 1);
        assert_eq!(diff.files[0].path, "README.md");
        let turns = db.lane_session_turns(&mapping.trail_session_id).unwrap();
        assert_eq!(turns.len(), 1);
        assert_ne!(Some(turns[0].before_change.clone()), turns[0].after_change);
        let view = db.agent_task_view("native-workspace-change").unwrap();
        assert_eq!(
            view.task.session_id.as_deref(),
            Some(mapping.trail_session_id.as_str())
        );
        assert_eq!(view.task.latest_checkpoint, Some(branch.head_change));
        assert_eq!(view.transcript.unwrap().turns.len(), 1);
    }

    #[test]
    fn artifact_content_is_bounded_addressed_and_scoped_to_the_exact_turn() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        db.spawn_lane(
            "artifact",
            None,
            false,
            Some("claude-code".to_string()),
            None,
        )
        .unwrap();
        let session = db
            .start_lane_session("artifact", Some("artifact session".to_string()), None)
            .unwrap()
            .session;
        let turn = db
            .begin_lane_session_turn("artifact", &session.session_id, None)
            .unwrap()
            .turn;
        let input = LaneArtifactInput {
            lane: "artifact".to_string(),
            session_id: session.session_id.clone(),
            turn_id: Some(turn.turn_id.clone()),
            provider: "claude-code".to_string(),
            artifact_kind: "native_transcript".to_string(),
            format: "jsonl".to_string(),
            source: AgentEvidenceSource::NativeTranscript,
            source_locator_redacted: Some("token=secret".to_string()),
            content: b"{\"role\":\"user\"}\n".to_vec(),
            start_offset: Some(0),
            end_offset: Some(16),
            redaction_profile: Some("trail-default-v1".to_string()),
            trust: "provider-native".to_string(),
            supersedes_artifact_id: None,
            metadata_json: Some("{\"fixture\":true}".to_string()),
        };
        let artifact = db.record_lane_artifact(input.clone()).unwrap();
        assert!(artifact.content_digest.starts_with("sha256:"));
        assert_eq!(
            artifact.source_locator_redacted.as_deref(),
            Some("token=[REDACTED]")
        );
        assert_eq!(
            db.record_lane_artifact(input).unwrap().artifact_id,
            artifact.artifact_id
        );
        assert_eq!(
            db.list_lane_artifacts(&session.session_id, Some(&turn.turn_id), 10)
                .unwrap(),
            vec![artifact]
        );
    }

    #[test]
    fn native_mapping_allocates_monotonic_receipt_sequences_and_one_finalizer() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        db.spawn_lane("hooks", None, false, Some("codex".to_string()), None)
            .unwrap();
        let session = db
            .start_lane_session(
                "hooks",
                Some("native hooks".to_string()),
                Some("session_hooks".to_string()),
            )
            .unwrap()
            .session;
        let mapping = db
            .ensure_lane_agent_session(LaneAgentSessionInput {
                provider: "codex".to_string(),
                native_session_id: "native-hooks-1".to_string(),
                parent_native_session_id: None,
                lane: "hooks".to_string(),
                trail_session_id: session.session_id,
                capture_run_id: None,
                primary_transport: AgentCaptureTransport::NativeHooks,
                transcript_identity: None,
            })
            .unwrap();

        let make_receipt = |dedupe_key: &str| AgentHookReceiptInput {
            installation_id: None,
            provider: "codex".to_string(),
            native_event: "event".to_string(),
            native_session_id: Some("native-hooks-1".to_string()),
            native_turn_id: None,
            transport: AgentCaptureTransport::NativeHooks,
            dedupe_key: dedupe_key.to_string(),
            payload: serde_json::json!({"key": dedupe_key}),
            occurred_at: None,
        };
        let first = db
            .persist_agent_hook_receipt(make_receipt("event:1"))
            .unwrap()
            .receipt;
        let second = db
            .persist_agent_hook_receipt(make_receipt("event:2"))
            .unwrap()
            .receipt;
        assert_eq!(
            db.assign_agent_hook_receipt_mapping(&first.receipt_id, &mapping.mapping_id)
                .unwrap()
                .receive_sequence,
            Some(1)
        );
        assert_eq!(
            db.assign_agent_hook_receipt_mapping(&second.receipt_id, &mapping.mapping_id)
                .unwrap()
                .receive_sequence,
            Some(2)
        );

        let first_lease = db
            .acquire_agent_finalization_lease(
                &mapping.mapping_id,
                "worker-1",
                30_000,
                AgentTurnOutcome::Failed,
                true,
            )
            .unwrap();
        assert!(first_lease.acquired);
        assert_eq!(first_lease.mapping.status, AgentCapturePhase::Finalizing);
        assert_eq!(
            first_lease.mapping.pending_turn_outcome,
            Some(AgentTurnOutcome::Failed)
        );
        let competing = db
            .acquire_agent_finalization_lease(
                &mapping.mapping_id,
                "worker-2",
                30_000,
                AgentTurnOutcome::Completed,
                false,
            )
            .unwrap();
        assert!(!competing.acquired);
        assert_eq!(
            competing.mapping.finalization_owner.as_deref(),
            Some("worker-1")
        );
    }

    #[test]
    fn managed_runs_choose_longest_workdir_and_reject_ambiguity() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        std::fs::create_dir_all(temp.path().join("apps/web")).unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();

        let outer = db
            .begin_agent_capture_run(AgentCaptureRunInput {
                lane: None,
                workdir: temp.path().to_string_lossy().to_string(),
                owner_agent: "codex".to_string(),
                owner_session_id: "outer".to_string(),
                executor_agent: None,
                work_item_id: None,
                lease_ms: 60_000,
                metadata_json: None,
            })
            .unwrap();
        let inner = db
            .begin_agent_capture_run(AgentCaptureRunInput {
                lane: None,
                workdir: temp.path().join("apps").to_string_lossy().to_string(),
                owner_agent: "codex".to_string(),
                owner_session_id: "inner".to_string(),
                executor_agent: None,
                work_item_id: Some("web".to_string()),
                lease_ms: 60_000,
                metadata_json: Some("{\"source\":\"test\"}".to_string()),
            })
            .unwrap();
        assert_eq!(
            db.match_agent_capture_run(temp.path().join("apps/web"), "codex")
                .unwrap()
                .unwrap()
                .capture_run_id,
            inner.capture_run_id
        );
        db.renew_agent_capture_run(&outer.capture_run_id, "codex", "outer", 120_000)
            .unwrap();

        db.begin_agent_capture_run(AgentCaptureRunInput {
            lane: None,
            workdir: temp.path().join("apps").to_string_lossy().to_string(),
            owner_agent: "codex".to_string(),
            owner_session_id: "ambiguous".to_string(),
            executor_agent: None,
            work_item_id: None,
            lease_ms: 60_000,
            metadata_json: None,
        })
        .unwrap();
        assert!(db
            .match_agent_capture_run(temp.path().join("apps/web"), "codex")
            .is_err());
        let ended = db
            .end_agent_capture_run(&outer.capture_run_id, "codex", "outer")
            .unwrap();
        assert_eq!(ended.status, "ended");
    }

    #[test]
    fn stale_receipts_recover_with_backoff_and_explicit_operator_resolution() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let receipt = db
            .persist_agent_hook_receipt(AgentHookReceiptInput {
                installation_id: None,
                provider: "codex".to_string(),
                native_event: "SessionStart".to_string(),
                native_session_id: Some("stale-session".to_string()),
                native_turn_id: None,
                transport: AgentCaptureTransport::NativeHooks,
                dedupe_key: "stale:1".to_string(),
                payload: serde_json::json!({"session_id":"stale-session"}),
                occurred_at: None,
            })
            .unwrap()
            .receipt;
        db.conn
            .execute(
                "UPDATE agent_hook_receipts SET status = 'processing', updated_at = 0
                 WHERE receipt_id = ?1",
                params![receipt.receipt_id],
            )
            .unwrap();
        assert_eq!(db.recover_stale_agent_hook_receipts(1_000).unwrap(), 1);
        assert_eq!(
            db.agent_hook_receipt(&receipt.receipt_id).unwrap().status,
            "retry"
        );
        assert_eq!(
            db.retry_agent_hook_receipt(&receipt.receipt_id)
                .unwrap()
                .status,
            "received"
        );
        assert_eq!(
            db.discard_agent_hook_receipt(&receipt.receipt_id)
                .unwrap()
                .status,
            "discarded"
        );
        assert!(db.retry_agent_hook_receipt(&receipt.receipt_id).is_err());
    }

    #[test]
    fn one_hundred_concurrent_duplicate_receipts_create_one_journal_row() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let workspace = std::sync::Arc::new(temp.path().to_path_buf());
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(100));
        let started = std::time::Instant::now();
        let handles = (0..100)
            .map(|_| {
                let workspace = std::sync::Arc::clone(&workspace);
                let barrier = std::sync::Arc::clone(&barrier);
                std::thread::spawn(move || {
                    let mut db = Trail::open(workspace.as_path()).unwrap();
                    barrier.wait();
                    Trail::with_write_lock_wait(Duration::from_secs(10), || {
                        db.persist_agent_hook_receipt(AgentHookReceiptInput {
                            installation_id: None,
                            provider: "codex".to_string(),
                            native_event: "SessionStart".to_string(),
                            native_session_id: Some("concurrent-session".to_string()),
                            native_turn_id: None,
                            transport: AgentCaptureTransport::NativeHooks,
                            dedupe_key: "concurrent:one".to_string(),
                            payload: serde_json::json!({"session_id":"concurrent-session"}),
                            occurred_at: None,
                        })
                    })
                    .unwrap()
                    .duplicate
                })
            })
            .collect::<Vec<_>>();
        let duplicate_count = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .filter(|duplicate| *duplicate)
            .count();
        assert_eq!(duplicate_count, 99);
        assert!(
            started.elapsed() < Duration::from_secs(15),
            "100 concurrent duplicate receipts exceeded the 15 second CI budget"
        );
        let db = Trail::open(workspace.as_path()).unwrap();
        assert_eq!(
            db.list_agent_hook_receipts(Some("codex"), None, 100)
                .unwrap()
                .len(),
            1
        );
        assert!(
            std::fs::metadata(workspace.join(".trail/index/trail.sqlite"))
                .unwrap()
                .len()
                < 32 * 1024 * 1024,
            "deduplicated ingress exceeded the 32 MiB database-growth budget"
        );
    }

    #[test]
    fn expired_managed_run_interrupts_open_turn_and_session() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        db.spawn_lane("expired", None, false, Some("codex".to_string()), None)
            .unwrap();
        let session = db
            .start_lane_session("expired", Some("expired run".to_string()), None)
            .unwrap()
            .session;
        let turn = db
            .begin_lane_session_turn("expired", &session.session_id, None)
            .unwrap()
            .turn;
        let run = db
            .begin_agent_capture_run(AgentCaptureRunInput {
                lane: Some("expired".to_string()),
                workdir: temp.path().to_string_lossy().to_string(),
                owner_agent: "codex".to_string(),
                owner_session_id: "owner-expired".to_string(),
                executor_agent: None,
                work_item_id: None,
                lease_ms: 60_000,
                metadata_json: None,
            })
            .unwrap();
        let mapping = db
            .ensure_lane_agent_session(LaneAgentSessionInput {
                provider: "codex".to_string(),
                native_session_id: "native-expired".to_string(),
                parent_native_session_id: None,
                lane: "expired".to_string(),
                trail_session_id: session.session_id.clone(),
                capture_run_id: Some(run.capture_run_id.clone()),
                primary_transport: AgentCaptureTransport::NativeHooks,
                transcript_identity: None,
            })
            .unwrap();
        db.conn
            .execute(
                "UPDATE lane_agent_sessions SET status = 'active' WHERE mapping_id = ?1",
                params![mapping.mapping_id],
            )
            .unwrap();
        db.conn
            .execute(
                "UPDATE agent_capture_runs SET expires_at = 0 WHERE capture_run_id = ?1",
                params![run.capture_run_id],
            )
            .unwrap();

        let report = db.reconcile_expired_agent_capture_runs().unwrap();
        assert_eq!(report.expired_run_ids, vec![run.capture_run_id]);
        assert_eq!(
            report.interrupted_mapping_ids,
            vec![mapping.mapping_id.clone()]
        );
        assert_eq!(report.interrupted_turn_ids, vec![turn.turn_id.clone()]);
        assert_eq!(db.lane_turn(&turn.turn_id).unwrap().status, "interrupted");
        assert_eq!(
            db.lane_session(&session.session_id).unwrap().status,
            "interrupted"
        );
        assert_eq!(
            db.lane_agent_session(&mapping.mapping_id).unwrap().status,
            AgentCapturePhase::Interrupted
        );
    }

    #[test]
    fn hybrid_native_receipts_enrich_acp_owned_turn_without_duplication() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        db.spawn_lane("hybrid", None, false, Some("codex".to_string()), None)
            .unwrap();
        let session = db
            .start_lane_session("hybrid", Some("hybrid".to_string()), None)
            .unwrap()
            .session;
        db.upsert_lane_acp_session(
            "acp-hybrid-session",
            Some("native-hybrid-session"),
            "hybrid",
            &session.session_id,
            &temp.path().to_string_lossy(),
            Some("codex"),
            None,
            None,
            "active",
        )
        .unwrap();
        let turn = db
            .begin_lane_session_turn("hybrid", &session.session_id, None)
            .unwrap()
            .turn;
        db.add_lane_turn_message(&turn.turn_id, "user", "same prompt")
            .unwrap();
        let prompt = db
            .persist_agent_hook_receipt(AgentHookReceiptInput {
                installation_id: None,
                provider: "codex".to_string(),
                native_event: "UserPromptSubmit".to_string(),
                native_session_id: Some("native-hybrid-session".to_string()),
                native_turn_id: Some("native-turn".to_string()),
                transport: AgentCaptureTransport::NativeHooks,
                dedupe_key: "hybrid:prompt".to_string(),
                payload: serde_json::json!({
                    "session_id":"native-hybrid-session",
                    "turn_id":"native-turn",
                    "prompt":"same prompt"
                }),
                occurred_at: None,
            })
            .unwrap()
            .receipt;
        let report = db.replay_agent_hook_receipt(&prompt.receipt_id).unwrap();
        let mapping = report.mapping.unwrap();
        assert_eq!(mapping.primary_transport, AgentCaptureTransport::Hybrid);
        assert_eq!(mapping.trail_session_id, session.session_id);
        assert_eq!(mapping.status, AgentCapturePhase::Idle);
        let details = db.show_lane_turn(&turn.turn_id).unwrap();
        assert_eq!(details.messages.len(), 1);
        assert_eq!(details.messages[0].body, "same prompt");
        assert!(details
            .events
            .iter()
            .any(|event| event.event_type == "turn.started"));
        assert!(db.lane_turn(&turn.turn_id).unwrap().ended_at.is_none());
    }

    #[test]
    fn canonical_export_precedes_transcript_and_missing_native_artifact_is_reconstructed() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        let transcript_path = temp.path().join("native.jsonl");
        let export_path = temp.path().join("canonical.json");
        std::fs::write(&transcript_path, b"native transcript").unwrap();
        std::fs::write(&export_path, b"{\"canonical\":true}").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        let persist = |db: &mut Trail,
                       session_id: &str,
                       event: &str,
                       suffix: &str,
                       payload: serde_json::Value| {
            db.persist_agent_hook_receipt(AgentHookReceiptInput {
                installation_id: None,
                provider: "opencode".to_string(),
                native_event: event.to_string(),
                native_session_id: Some(session_id.to_string()),
                native_turn_id: None,
                transport: AgentCaptureTransport::NativeHooks,
                dedupe_key: format!("{session_id}:{suffix}"),
                payload,
                occurred_at: None,
            })
            .unwrap()
            .receipt
            .receipt_id
        };

        let session_id = "opencode-canonical";
        let started = persist(
            &mut db,
            session_id,
            "session.created",
            "start",
            serde_json::json!({
                "sessionID": session_id,
                "transcript_path": transcript_path,
            }),
        );
        let prompt = persist(
            &mut db,
            session_id,
            "chat.message",
            "prompt",
            serde_json::json!({"sessionID":session_id,"message":"do it"}),
        );
        let ended = persist(
            &mut db,
            session_id,
            "session.idle",
            "end",
            serde_json::json!({
                "sessionID":session_id,
                "canonical_export_path": export_path,
            }),
        );
        db.replay_agent_hook_receipt(&started).unwrap();
        db.replay_agent_hook_receipt(&prompt).unwrap();
        let mapping = db
            .replay_agent_hook_receipt(&ended)
            .unwrap()
            .mapping
            .unwrap();
        let artifacts = db
            .list_lane_artifacts(&mapping.trail_session_id, None, 10)
            .unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact_kind, "export");
        assert_eq!(artifacts[0].source, AgentEvidenceSource::CanonicalExport);
        assert_eq!(
            db.lane_artifact_content(&artifacts[0].artifact_id).unwrap(),
            b"{\"canonical\":true}"
        );
        std::fs::write(&export_path, b"{\"canonical\":false}").unwrap();
        let refreshed = db.replay_agent_hook_receipt(&ended).unwrap();
        assert!(!refreshed.replayed);
        assert_eq!(
            refreshed.actions,
            vec![AgentCaptureAction::ImportTranscript]
        );
        let refreshed_artifacts = db
            .list_lane_artifacts(&mapping.trail_session_id, None, 10)
            .unwrap();
        assert_eq!(refreshed_artifacts.len(), 2);
        assert!(refreshed_artifacts
            .iter()
            .all(|artifact| artifact.turn_id.is_some()));
        assert_eq!(
            db.lane_artifact_content(&refreshed_artifacts[0].artifact_id)
                .unwrap(),
            b"{\"canonical\":false}"
        );
        std::fs::write(&export_path, b"{}").unwrap();
        db.replay_agent_hook_receipt(&ended).unwrap();
        let truncated = db
            .list_lane_artifacts(&mapping.trail_session_id, None, 10)
            .unwrap();
        assert_eq!(truncated.len(), 3);
        let metadata: serde_json::Value =
            serde_json::from_str(truncated[0].metadata_json.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["truncated"], true);
        assert_eq!(truncated[0].end_offset, Some(2));

        let reconstructed_session = "opencode-reconstructed";
        let prompt = persist(
            &mut db,
            reconstructed_session,
            "chat.message",
            "prompt",
            serde_json::json!({"sessionID":reconstructed_session,"message":"do it"}),
        );
        let ended = persist(
            &mut db,
            reconstructed_session,
            "session.idle",
            "end",
            serde_json::json!({"sessionID":reconstructed_session}),
        );
        db.replay_agent_hook_receipt(&prompt).unwrap();
        let mapping = db
            .replay_agent_hook_receipt(&ended)
            .unwrap()
            .mapping
            .unwrap();
        let artifacts = db
            .list_lane_artifacts(&mapping.trail_session_id, None, 10)
            .unwrap();
        assert_eq!(artifacts.len(), 1);
        assert_eq!(artifacts[0].artifact_kind, "transcript");
        assert_eq!(artifacts[0].source, AgentEvidenceSource::Reconstructed);
        assert!(
            String::from_utf8(db.lane_artifact_content(&artifacts[0].artifact_id).unwrap())
                .unwrap()
                .contains("trail.reconstructed_transcript")
        );
    }

    #[cfg(unix)]
    #[test]
    fn transcript_import_rejects_leaf_symlinks_and_paths_outside_approved_roots() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("README.md"), "hello\n").unwrap();
        Trail::init(temp.path(), "main", InitImportMode::WorkingTree, false).unwrap();
        let db = Trail::open(temp.path()).unwrap();
        let target = temp.path().join("transcript.jsonl");
        let link = temp.path().join("transcript-link.jsonl");
        std::fs::write(&target, "{}\n").unwrap();
        symlink(&target, &link).unwrap();

        assert!(reject_agent_transcript_symlinks(&link).is_err());
        assert!(!db
            .agent_transcript_path_allowed("opencode", std::path::Path::new("/etc/hosts"))
            .unwrap());
    }
}
