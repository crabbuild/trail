use super::initialization_owner::{owner_fence_matches, LaneInitializationFence};
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

#[derive(serde::Serialize)]
struct CanonicalLaneSpawnRequestV1<'a> {
    version: u8,
    workspace_id: &'a str,
    lane_name: &'a str,
    source_ref: &'a str,
    source_change: &'a str,
    source_root: &'a str,
    requested_workdir_mode: &'a LaneWorkdirMode,
    workdir: Option<&'a str>,
    sparse_paths: &'a [String],
    include_neighbors: bool,
    provider: Option<&'a str>,
    model: Option<&'a str>,
}

impl CanonicalLaneSpawnRequestV1<'_> {
    fn fingerprint(&self) -> Result<String> {
        let bytes = serde_json::to_vec(self)?;
        Ok(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
    }
}

#[derive(Clone, Debug)]
pub(crate) struct ResolvedLaneSpawnRequest {
    pub lane_name: String,
    pub lane_id: String,
    pub source_ref: String,
    pub source_change: ChangeId,
    pub source_root: ObjectId,
    pub source_operation: ObjectId,
    pub requested_workdir_mode: LaneWorkdirMode,
    pub workdir: Option<PathBuf>,
    pub sparse_paths: Vec<String>,
    pub include_neighbors: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub request_fingerprint: String,
    pub initialization_id: String,
}

impl ResolvedLaneSpawnRequest {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        workspace_id: &str,
        lane_name: &str,
        lane_id: String,
        source_ref: String,
        source_change: ChangeId,
        source_root: ObjectId,
        source_operation: ObjectId,
        requested_workdir_mode: LaneWorkdirMode,
        workdir: Option<PathBuf>,
        sparse_paths: Vec<String>,
        include_neighbors: bool,
        provider: Option<String>,
        model: Option<String>,
    ) -> Result<Self> {
        let workdir_text = workdir
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned());
        let canonical = CanonicalLaneSpawnRequestV1 {
            version: 1,
            workspace_id,
            lane_name,
            source_ref: &source_ref,
            source_change: &source_change.0,
            source_root: &source_root.0,
            requested_workdir_mode: &requested_workdir_mode,
            workdir: workdir_text.as_deref(),
            sparse_paths: &sparse_paths,
            include_neighbors,
            provider: provider.as_deref(),
            model: model.as_deref(),
        };
        let request_fingerprint = canonical.fingerprint()?;
        let mut digest = Sha256::new();
        digest.update(b"trail-lane-initialization-v1\0");
        digest.update(workspace_id.as_bytes());
        digest.update([0]);
        digest.update(lane_name.as_bytes());
        digest.update([0]);
        digest.update(request_fingerprint.as_bytes());
        let initialization_id = format!("init_{}", hex::encode(digest.finalize()));
        Ok(Self {
            lane_name: lane_name.to_string(),
            lane_id,
            source_ref,
            source_change,
            source_root,
            source_operation,
            requested_workdir_mode,
            workdir,
            sparse_paths,
            include_neighbors,
            provider,
            model,
            request_fingerprint,
            initialization_id,
        })
    }
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

pub(crate) fn lane_initialization_record(
    conn: &Connection,
    lane: &str,
) -> Result<Option<LaneInitializationRecord>> {
    conn.query_row(
        "SELECT initialization_id,lane_name,lane_id,request_fingerprint,
                operation_id,phase,workdir,materialization_json,last_error_code,
                last_error_message,repair_command,created_at,updated_at
         FROM lane_initializations
         WHERE lane_name=?1 OR lane_id=?1 OR initialization_id=?1
         ORDER BY CASE WHEN lane_name=?1 THEN 0 ELSE 1 END
         LIMIT 1",
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
            })
        },
    )
    .transpose()
}

pub(crate) fn insert_lane_initialization_reservation(
    conn: &Connection,
    request: &ResolvedLaneSpawnRequest,
) -> Result<LaneInitializationRecord> {
    let lane_ref_exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM refs WHERE name=?1)",
        [lane_ref(&request.lane_name)],
        |row| row.get(0),
    )?;
    if lane_ref_exists {
        return Err(Error::InvalidInput(format!(
            "lane `{}` already exists without initialization identity",
            request.lane_name
        )));
    }

    let now = now_ts();
    conn.execute(
        "INSERT INTO lane_initializations(
             initialization_id,lane_name,lane_id,request_fingerprint,operation_id,
             phase,workdir,materialization_json,last_error_code,last_error_message,
             repair_command,created_at,updated_at)
         VALUES(?1,?2,?3,?4,?5,'reserved',?6,NULL,NULL,NULL,NULL,?7,?7)",
        params![
            request.initialization_id,
            request.lane_name,
            request.lane_id,
            request.request_fingerprint,
            request.source_operation.0,
            request
                .workdir
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
            now,
        ],
    )?;
    lane_initialization_record(conn, &request.lane_name)?
        .ok_or_else(|| Error::Corrupt("lane initialization reservation disappeared".into()))
}

pub(crate) struct LaneInitializationUpdate<'a> {
    pub(crate) operation_id: Option<&'a str>,
    pub(crate) workdir: Option<&'a Path>,
    pub(crate) materialization_json: Option<&'a str>,
    pub(crate) last_error: Option<&'a Error>,
}

impl LaneInitializationUpdate<'_> {
    pub(crate) fn none() -> Self {
        Self {
            operation_id: None,
            workdir: None,
            materialization_json: None,
            last_error: None,
        }
    }
}

fn phase_database_name(phase: LaneInitializationPhase) -> &'static str {
    match phase {
        LaneInitializationPhase::Reserved => "reserved",
        LaneInitializationPhase::Materialized => "materialized",
        LaneInitializationPhase::Associated => "associated",
        LaneInitializationPhase::ObserverReady => "observer_ready",
        LaneInitializationPhase::RepairRequired => "repair_required",
    }
}

pub(crate) fn transition_lane_initialization(
    tx: &rusqlite::Transaction<'_>,
    initialization_id: &str,
    fence: &LaneInitializationFence,
    expected: LaneInitializationPhase,
    next: LaneInitializationPhase,
    update: LaneInitializationUpdate<'_>,
) -> Result<()> {
    let error_code = update.last_error.map(Error::code);
    let error_message = update.last_error.map(|error| {
        let mut message = error.to_string();
        if message.len() > 4096 {
            let mut boundary = 4096;
            while !message.is_char_boundary(boundary) {
                boundary -= 1;
            }
            message.truncate(boundary);
        }
        message
    });
    let changed = tx.execute(
        "UPDATE lane_initializations
         SET phase=?1,
             operation_id=COALESCE(?2,operation_id),
             workdir=COALESCE(?3,workdir),
             materialization_json=COALESCE(?4,materialization_json),
             last_error_code=?5,last_error_message=?6,
             repair_command=CASE WHEN ?1='repair_required'
               THEN 'trail lane repair-initialization ' || lane_name ELSE NULL END,
             updated_at=?7
         WHERE initialization_id=?8 AND phase=?9
           AND EXISTS(
             SELECT 1 FROM lane_initialization_owners owner
             WHERE owner.initialization_id=lane_initializations.initialization_id
               AND owner.owner_token=?10 AND owner.owner_generation=?11)",
        params![
            phase_database_name(next),
            update.operation_id,
            update
                .workdir
                .map(|path| path.to_string_lossy().into_owned()),
            update.materialization_json,
            error_code,
            error_message,
            now_ts(),
            initialization_id,
            phase_database_name(expected),
            fence.owner_token,
            fence.owner_generation,
        ],
    )?;
    if changed == 1 {
        let owner_changed = if matches!(
            next,
            LaneInitializationPhase::ObserverReady | LaneInitializationPhase::RepairRequired
        ) {
            tx.execute(
                "DELETE FROM lane_initialization_owners
                 WHERE initialization_id=?1 AND owner_token=?2 AND owner_generation=?3",
                params![initialization_id, fence.owner_token, fence.owner_generation],
            )?
        } else {
            tx.execute(
                "UPDATE lane_initialization_owners SET heartbeat_at=?1
                 WHERE initialization_id=?2 AND owner_token=?3 AND owner_generation=?4",
                params![
                    now_ts(),
                    initialization_id,
                    fence.owner_token,
                    fence.owner_generation,
                ],
            )?
        };
        if owner_changed != 1 {
            return Err(lane_initialization_ownership_lost(initialization_id));
        }
        return Ok(());
    }
    let current = lane_initialization_record(tx, initialization_id)?.ok_or_else(|| {
        Error::Corrupt(format!(
            "lane initialization `{initialization_id}` disappeared during transition"
        ))
    })?;
    let fence_matches = owner_fence_matches(tx, initialization_id, fence)?;
    let any_owner: bool = tx.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM lane_initialization_owners WHERE initialization_id=?1)",
        [initialization_id],
        |row| row.get(0),
    )?;
    let terminal_without_owner = matches!(
        next,
        LaneInitializationPhase::ObserverReady | LaneInitializationPhase::RepairRequired
    ) && !any_owner;
    if current.phase == next && (fence_matches || terminal_without_owner) {
        return Ok(());
    }
    if !fence_matches {
        return Err(lane_initialization_ownership_lost(initialization_id));
    }
    Err(Error::Corrupt(format!(
        "lane initialization `{initialization_id}` is {:?}, expected {:?} for transition to {:?}",
        current.phase, expected, next
    )))
}

fn lane_initialization_ownership_lost(initialization_id: &str) -> Error {
    Error::LaneInitializationOwnershipLost {
        initialization_id: initialization_id.to_string(),
    }
}

impl Trail {
    pub fn lane_initialization(&self, lane: &str) -> Result<Option<LaneInitializationReport>> {
        Ok(lane_initialization_record(&self.conn, lane)?.map(LaneInitializationRecord::report))
    }

    pub(crate) fn mark_lane_initialization_materialized(
        &mut self,
        request: &ResolvedLaneSpawnRequest,
        fence: &LaneInitializationFence,
        operation_id: &ObjectId,
        materialization: Option<&MaterializationReport>,
    ) -> Result<()> {
        let materialization_json = materialization.map(serde_json::to_string).transpose()?;
        let tx = self.conn.transaction()?;
        transition_lane_initialization(
            &tx,
            &request.initialization_id,
            fence,
            LaneInitializationPhase::Reserved,
            LaneInitializationPhase::Materialized,
            LaneInitializationUpdate {
                operation_id: Some(&operation_id.0),
                workdir: request.workdir.as_deref(),
                materialization_json: materialization_json.as_deref(),
                last_error: None,
            },
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn mark_lane_initialization_observer_ready(
        &mut self,
        request: &ResolvedLaneSpawnRequest,
        fence: &LaneInitializationFence,
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        transition_lane_initialization(
            &tx,
            &request.initialization_id,
            fence,
            LaneInitializationPhase::Associated,
            LaneInitializationPhase::ObserverReady,
            LaneInitializationUpdate::none(),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn mark_lane_initialization_repair_required(
        &mut self,
        initialization_id: &str,
        fence: &LaneInitializationFence,
        error: &Error,
    ) -> Result<LaneInitializationRecord> {
        let current =
            lane_initialization_record(&self.conn, initialization_id)?.ok_or_else(|| {
                Error::Corrupt(format!(
                    "lane initialization `{initialization_id}` disappeared"
                ))
            })?;
        if matches!(
            current.phase,
            LaneInitializationPhase::ObserverReady | LaneInitializationPhase::RepairRequired
        ) {
            let has_owner: bool = self.conn.query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM lane_initialization_owners WHERE initialization_id=?1)",
                [initialization_id],
                |row| row.get(0),
            )?;
            if current.phase == LaneInitializationPhase::RepairRequired && has_owner {
                let tx = self.conn.transaction()?;
                transition_lane_initialization(
                    &tx,
                    initialization_id,
                    fence,
                    LaneInitializationPhase::RepairRequired,
                    LaneInitializationPhase::RepairRequired,
                    LaneInitializationUpdate {
                        last_error: Some(error),
                        ..LaneInitializationUpdate::none()
                    },
                )?;
                tx.commit()?;
                return lane_initialization_record(&self.conn, initialization_id)?.ok_or_else(
                    || {
                        Error::Corrupt(format!(
                            "lane initialization `{initialization_id}` disappeared"
                        ))
                    },
                );
            }
            if has_owner {
                return Err(lane_initialization_ownership_lost(initialization_id));
            }
            return Ok(current);
        }
        if current.phase != LaneInitializationPhase::Associated {
            return Err(Error::Corrupt(format!(
                "cannot require repair for unassociated lane initialization `{initialization_id}`"
            )));
        }
        let tx = self.conn.transaction()?;
        transition_lane_initialization(
            &tx,
            initialization_id,
            fence,
            current.phase,
            LaneInitializationPhase::RepairRequired,
            LaneInitializationUpdate {
                last_error: Some(error),
                ..LaneInitializationUpdate::none()
            },
        )?;
        tx.commit()?;
        lane_initialization_record(&self.conn, initialization_id)?.ok_or_else(|| {
            Error::Corrupt(format!(
                "lane initialization `{initialization_id}` disappeared"
            ))
        })
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn debug_transition_lane_initialization_with_fence(
        &mut self,
        initialization_id: &str,
        owner_token: &str,
        owner_generation: i64,
        expected: LaneInitializationPhase,
        next: LaneInitializationPhase,
    ) -> Result<()> {
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        transition_lane_initialization(
            &tx,
            initialization_id,
            &LaneInitializationFence {
                owner_token: owner_token.to_string(),
                owner_generation,
            },
            expected,
            next,
            LaneInitializationUpdate::none(),
        )?;
        tx.commit()?;
        Ok(())
    }

    pub(crate) fn complete_deferred_lane_initialization_owned(
        &mut self,
        lane: &str,
        fence: &LaneInitializationFence,
    ) -> Result<LaneInitializationRecord> {
        let record = lane_initialization_record(&self.conn, lane)?
            .ok_or_else(|| Error::Corrupt(format!("lane `{lane}` has no initialization row")))?;
        if record.phase == LaneInitializationPhase::ObserverReady {
            return Ok(record);
        }
        if !matches!(
            record.phase,
            LaneInitializationPhase::Associated | LaneInitializationPhase::RepairRequired
        ) {
            return Err(Error::Corrupt(format!(
                "lane initialization `{}` is {:?}, expected associated",
                record.initialization_id, record.phase
            )));
        }
        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        transition_lane_initialization(
            &tx,
            &record.initialization_id,
            fence,
            record.phase,
            LaneInitializationPhase::ObserverReady,
            LaneInitializationUpdate::none(),
        )?;
        tx.commit()?;
        lane_initialization_record(&self.conn, lane)?
            .ok_or_else(|| Error::Corrupt(format!("lane `{lane}` initialization disappeared")))
    }
}
