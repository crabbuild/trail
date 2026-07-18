use super::*;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LaneInitializationRecord {
    pub initialization_id: String,
    pub lane_name: String,
    pub lane_id: String,
    pub request_fingerprint: String,
    pub operation_id: String,
    pub phase: LaneInitializationPhase,
    pub workdir: Option<PathBuf>,
    pub materialization_json: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub repair_command: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl LaneInitializationPhase {
    fn from_database(value: &str) -> Result<Self> {
        serde_json::from_value(serde_json::Value::String(value.to_string()))
            .map_err(|_| Error::Corrupt(format!("invalid lane initialization phase `{value}`")))
    }
}

impl LaneInitializationRecord {
    fn report(self) -> LaneInitializationReport {
        LaneInitializationReport {
            initialization_id: self.initialization_id,
            lane_name: self.lane_name,
            lane_id: self.lane_id,
            request_fingerprint: self.request_fingerprint,
            operation_id: self.operation_id,
            phase: self.phase,
            workdir: self.workdir.map(|path| path.to_string_lossy().into_owned()),
            last_error_code: self.last_error_code,
            last_error_message: self.last_error_message,
            repair_command: self.repair_command,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

impl Trail {
    pub fn lane_initialization(&self, lane: &str) -> Result<Option<LaneInitializationReport>> {
        self.conn
            .query_row(
                "SELECT initialization_id,lane_name,lane_id,request_fingerprint,
                        operation_id,phase,workdir,materialization_json,last_error_code,
                        last_error_message,repair_command,created_at,updated_at
                 FROM lane_initializations
                 WHERE lane_name=?1 OR lane_id=?1",
                params![lane],
                |row| {
                    let phase = row.get::<_, String>(5)?;
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        phase,
                        row.get::<_, Option<String>>(6)?,
                        row.get(7)?,
                        row.get(8)?,
                        row.get(9)?,
                        row.get(10)?,
                        row.get(11)?,
                        row.get(12)?,
                    ))
                },
            )
            .optional()?
            .map(
                |(
                    initialization_id,
                    lane_name,
                    lane_id,
                    request_fingerprint,
                    operation_id,
                    phase,
                    workdir,
                    materialization_json,
                    last_error_code,
                    last_error_message,
                    repair_command,
                    created_at,
                    updated_at,
                )| {
                    Ok(LaneInitializationRecord {
                        initialization_id,
                        lane_name,
                        lane_id,
                        request_fingerprint,
                        operation_id,
                        phase: LaneInitializationPhase::from_database(&phase)?,
                        workdir: workdir.map(PathBuf::from),
                        materialization_json,
                        last_error_code,
                        last_error_message,
                        repair_command,
                        created_at,
                        updated_at,
                    }
                    .report())
                },
            )
            .transpose()
    }
}

pub(crate) fn backfill_lane_initializations_v19(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    let lanes = tx
        .prepare(
            "SELECT lane.name,lane.lane_id,branch.base_change,branch.base_root,
                    branch.workdir,lane.metadata_json,lane.provider,lane.model,
                    ref.operation_id,branch.created_at,branch.updated_at
             FROM lane_branches branch
             JOIN lanes lane ON lane.lane_id=branch.lane_id
             LEFT JOIN refs ref ON ref.name=branch.ref_name
             WHERE branch.status='active'
             ORDER BY lane.name",
        )?
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, i64>(9)?,
                row.get::<_, i64>(10)?,
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()?;

    for (
        lane_name,
        lane_id,
        base_change,
        base_root,
        workdir,
        metadata_json,
        provider,
        model,
        operation_id,
        created_at,
        updated_at,
    ) in lanes
    {
        let metadata = metadata_json
            .as_deref()
            .and_then(|value| serde_json::from_str::<serde_json::Value>(value).ok());
        let fingerprint_value = serde_json::json!({
            "version": "legacy_lane_initialization_v1",
            "lane_name": lane_name,
            "base_change": base_change,
            "base_root": base_root,
            "requested_workdir_mode": metadata.as_ref().and_then(|value| value.get("requested_workdir_mode")),
            "workdir_mode": metadata.as_ref().and_then(|value| value.get("workdir_mode")),
            "workdir": workdir,
            "sparse_paths": metadata.as_ref().and_then(|value| value.get("sparse_paths")),
            "include_neighbors": metadata.as_ref().and_then(|value| value.get("include_neighbors")),
            "provider": provider,
            "model": model,
        });
        let fingerprint_bytes = serde_json::to_vec(&fingerprint_value)?;
        let mut fingerprint_digest = Sha256::new();
        fingerprint_digest.update(b"trail-legacy-lane-initialization-fingerprint-v1\0");
        fingerprint_digest.update(fingerprint_bytes);
        let request_fingerprint = hex::encode(fingerprint_digest.finalize());
        let mut initialization_digest = Sha256::new();
        initialization_digest.update(b"trail-legacy-lane-initialization-v1\0");
        initialization_digest.update(lane_name.as_bytes());
        initialization_digest.update([0]);
        initialization_digest.update(request_fingerprint.as_bytes());
        let initialization_id = format!("init_{}", hex::encode(initialization_digest.finalize()));
        let phase = backfilled_lane_phase(tx, &lane_id, workdir.as_deref())?;
        let failure = backfilled_lane_failed_invariant(tx, &lane_id, workdir.as_deref())?;
        let phase_text = match phase {
            LaneInitializationPhase::ObserverReady => "observer_ready",
            LaneInitializationPhase::RepairRequired => "repair_required",
            _ => unreachable!("legacy backfill only writes terminal phases"),
        };
        let materialization_json = metadata
            .as_ref()
            .and_then(|value| value.get("materialization"))
            .filter(|value| !value.is_null())
            .map(serde_json::to_string)
            .transpose()?;
        let repair_command = failure
            .as_ref()
            .map(|_| format!("trail lane repair-initialization {lane_name}"));
        tx.execute(
            "INSERT INTO lane_initializations(
                 initialization_id,lane_name,lane_id,request_fingerprint,operation_id,
                 phase,workdir,materialization_json,last_error_code,last_error_message,
                 repair_command,created_at,updated_at)
             VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)",
            params![
                initialization_id,
                lane_name,
                lane_id,
                request_fingerprint,
                operation_id.unwrap_or_else(|| format!("legacy_{lane_id}")),
                phase_text,
                workdir,
                materialization_json,
                failure.as_ref().map(|_| "LANE_INITIALIZATION_INCOMPLETE"),
                failure,
                repair_command,
                created_at,
                updated_at,
            ],
        )?;
    }
    Ok(())
}

fn backfilled_lane_phase(
    tx: &rusqlite::Transaction<'_>,
    lane_id: &str,
    workdir: Option<&str>,
) -> Result<LaneInitializationPhase> {
    Ok(
        if backfilled_lane_failed_invariant(tx, lane_id, workdir)?.is_none() {
            LaneInitializationPhase::ObserverReady
        } else {
            LaneInitializationPhase::RepairRequired
        },
    )
}

fn backfilled_lane_failed_invariant(
    tx: &rusqlite::Transaction<'_>,
    lane_id: &str,
    workdir: Option<&str>,
) -> Result<Option<String>> {
    let association_matches: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM lane_branches branch
             JOIN refs ref ON ref.name=branch.ref_name
             WHERE branch.lane_id=?1 AND branch.status='active'
               AND ref.change_id=branch.head_change AND ref.root_id=branch.head_root)",
        params![lane_id],
        |row| row.get(0),
    )?;
    if !association_matches {
        return Ok(Some(
            "lane ref does not match the active branch head".into(),
        ));
    }
    if workdir.is_some() {
        let materialization_complete: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM lanes
                 WHERE lane_id=?1 AND json_valid(metadata_json)
                   AND json_type(metadata_json,'$.requested_workdir_mode')='text'
                   AND json_type(metadata_json,'$.workdir_mode')='text'
                   AND json_type(metadata_json,'$.workdir_backend')='text'
                   AND json_type(metadata_json,'$.materialization')='object'
                   AND json_type(metadata_json,'$.sparse_paths')='array')",
            params![lane_id],
            |row| row.get(0),
        )?;
        if !materialization_complete {
            return Ok(Some(
                "materialized lane metadata is missing or incomplete".into(),
            ));
        }
        let clean_checkpoint: bool = tx.query_row(
            "SELECT EXISTS(
                 SELECT 1 FROM changed_path_scopes scope
                 JOIN lane_branches branch ON branch.lane_id=scope.owner_id
                 JOIN refs ref ON ref.name=branch.ref_name
                 JOIN changed_path_observer_owners owner
                   ON owner.scope_id=scope.scope_id AND owner.epoch=scope.epoch
                 JOIN changed_path_observer_segments segment
                   ON segment.scope_id=scope.scope_id AND segment.epoch=scope.epoch
                 WHERE scope.scope_kind='materialized_lane' AND scope.owner_id=?1
                   AND scope.scope_root=?2
                   AND scope.ref_name=branch.ref_name
                   AND scope.ref_generation=ref.generation
                   AND scope.change_id=branch.head_change
                   AND scope.baseline_root_id=branch.head_root
                   AND scope.trust_state='trusted' AND scope.clean_proof_allowed=1
                   AND scope.linearizable_fence=1 AND scope.filesystem_supported=1
                   AND scope.power_loss_durability=1
                   AND scope.durable_offset=scope.folded_offset
                   AND owner.lease_state='active' AND owner.error_state IS NULL
                   AND owner.error_at IS NULL AND owner.expires_at>?3
                   AND owner.provider_id=scope.provider_id
                   AND owner.provider_identity=scope.provider_identity
                   AND scope.observer_owner_token=owner.owner_token
                   AND segment.owner_token=owner.owner_token
                   AND segment.provider_id=owner.provider_id
                   AND segment.folded_end_offset=scope.folded_offset
                   AND segment.durable_end_offset>=scope.durable_offset
                   AND segment.state IN ('open','sealed')
                   AND scope.retired_at IS NULL)",
            params![lane_id, workdir, now_ts()],
            |row| row.get(0),
        )?;
        if !clean_checkpoint {
            return Ok(Some(
                "materialized lane clean checkpoint/observer scope is missing or inconsistent"
                    .into(),
            ));
        }
    }
    let spawned: bool = tx.query_row(
        "SELECT EXISTS(SELECT 1 FROM lane_events
                       WHERE lane_id=?1 AND event_type='lane_spawned')",
        params![lane_id],
        |row| row.get(0),
    )?;
    if !spawned {
        return Ok(Some("lane_spawned event is missing".into()));
    }
    Ok(None)
}
