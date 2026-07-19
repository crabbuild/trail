use super::*;
#[cfg(any(test, debug_assertions))]
use std::collections::VecDeque;
#[cfg(any(test, debug_assertions))]
thread_local! {
    static SCHEMA_V19_BACKFILL_TIMES: std::cell::RefCell<VecDeque<i64>> =
        const { std::cell::RefCell::new(VecDeque::new()) };
}

#[cfg(any(test, debug_assertions))]
pub(crate) fn install_schema_v19_backfill_times(times: Vec<i64>) {
    SCHEMA_V19_BACKFILL_TIMES.with(|installed| *installed.borrow_mut() = times.into());
}

#[cfg(any(test, debug_assertions))]
pub(crate) fn clear_schema_v19_backfill_times() {
    SCHEMA_V19_BACKFILL_TIMES.with(|installed| installed.borrow_mut().clear());
}

#[cfg(any(test, debug_assertions))]
pub(crate) fn schema_v19_backfill_times_remaining() -> usize {
    SCHEMA_V19_BACKFILL_TIMES.with(|installed| installed.borrow().len())
}

#[cfg(any(test, debug_assertions))]
pub(crate) fn install_schema_v18_authenticated_lane_evidence(
    workspace: &Path,
    lane_id: &str,
) -> Result<()> {
    let workspace = canonicalize_lossless(workspace)?;
    let database_path = workspace.join(".trail/index/trail.sqlite");
    let workdir;
    let ref_name;
    let ref_generation;
    let head_change;
    let head_root;
    {
        let conn = Connection::open(&database_path)?;
        (workdir, ref_name, ref_generation, head_change, head_root) = conn.query_row(
            "SELECT branch.workdir,branch.ref_name,ref.generation,
                    branch.head_change,branch.head_root
             FROM lane_branches branch
             JOIN refs ref ON ref.name=branch.ref_name
             WHERE branch.lane_id=?1 AND branch.status='active'",
            [lane_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                ))
            },
        )?;
    }
    let workdir = canonicalize_lossless(Path::new(&workdir))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(workdir.join(".trail"), fs::Permissions::from_mode(0o700))?;
    }
    Connection::open(&database_path)?.execute(
        "UPDATE lane_branches SET workdir=?1 WHERE lane_id=?2 AND status='active'",
        params![workdir.to_string_lossy().as_ref(), lane_id],
    )?;
    let filesystem_identity = super::workdir::materialized_lane_root_identity(&workdir)
        .map_err(|error| Error::Corrupt(format!("fixture root identity: {error}")))?;
    let filesystem_identity_text = hex::encode(&filesystem_identity);
    let workspace_id = WorkspaceId::new(workspace.to_string_lossy().as_bytes());
    let scope_id = crate::db::change_ledger::materialized_lane_scope_id(&workspace_id.0, lane_id);
    let scope_text = scope_id.to_text();
    let provider_identity = [0x77_u8; 32];
    let provider_text = hex::encode(provider_identity);
    let policy_fingerprint = [0xaa_u8; 32];
    let policy_text = hex::encode(policy_fingerprint);
    let now = now_ts();
    {
        let conn = Connection::open(&database_path)?;
        crate::db::util::apply_sqlite_pragmas(&conn)?;
        conn.execute(
            "INSERT INTO changed_path_scopes(
                 scope_id,scope_kind,owner_id,scope_root,scope_root_identity,
                 filesystem_identity,filesystem_kind,case_sensitive,ref_name,
                 ref_generation,change_id,baseline_root_id,policy_fingerprint,
                 policy_dependency_generation,trust_state,trust_reason,epoch,
                 provider_id,provider_identity,durable_cursor,linearizable_fence,
                 rename_pairing,overflow_scope,filesystem_supported,clean_proof_allowed,
                 power_loss_durability,durable_offset,folded_offset,created_at,updated_at)
             VALUES(?1,'materialized_lane',?2,?3,?4,?4,'native',1,?5,?6,?7,?8,?9,1,
                    'reconciling','schema18_authenticated_fixture',1,?10,?10,1,1,1,0,1,1,1,
                    0,0,?11,?11)",
            params![
                scope_text,
                lane_id,
                workdir.to_string_lossy().as_ref(),
                filesystem_identity_text,
                ref_name,
                ref_generation,
                head_change,
                head_root,
                policy_text,
                provider_text,
                now,
            ],
        )?;
        conn.execute(
            "INSERT INTO changed_path_policy_dependencies(
                 scope_id,dependency_identity,dependency_kind,content_identity,
                 metadata_identity,observable,generation,last_source_sequence,
                 created_at,updated_at)
             VALUES(?1,'builtin:recording-policy','builtin',?2,?3,1,1,1,?4,?4)",
            params![scope_text, vec![0x11_u8; 32], vec![0x22_u8; 32], now],
        )?;
    }
    let owner_token = [0xbb_u8; 32];
    let segment_directory = workspace.join(".trail/observer-segments").join(&scope_text);
    let mut writer = crate::db::change_ledger::SegmentWriter::acquire(
        &database_path,
        &segment_directory,
        scope_id,
        1,
        owner_token,
        &provider_text,
        Vec::new(),
        std::time::Duration::from_secs(3_600),
    )
    .map_err(|error| Error::Corrupt(format!("fixture segment acquire: {error}")))?;
    let cut = writer
        .append_and_flush(&[crate::db::change_ledger::ObserverRecord {
            sequence: 1,
            source: crate::db::change_ledger::EvidenceSource::Observer,
            path: crate::db::change_ledger::LedgerPath::parse("authenticated.txt")?,
            flags: crate::db::change_ledger::EvidenceFlags::CREATE,
            provider_cursor: vec![1],
        }])
        .map_err(|error| Error::Corrupt(format!("fixture segment append: {error}")))?;
    drop(writer);
    let owner_text = hex::encode(owner_token);
    let process_start_identity = crate::db::util::process_liveness::current_process_start_token();
    let conn = Connection::open(&database_path)?;
    conn.execute(
        "UPDATE changed_path_observer_owners
         SET provider_identity=?1,fence_nonce=?2,
             daemon_launch_nonce=?3,daemon_pid=?4,
             daemon_process_start_identity=?5,updated_at=?6
         WHERE scope_id=?7 AND epoch=1 AND owner_token=?8",
        params![
            provider_text,
            vec![0x33_u8; 24],
            hex::encode([0xcc_u8; 32]),
            i64::from(std::process::id()),
            process_start_identity,
            now_ts(),
            scope_text,
            owner_text,
        ],
    )?;
    conn.execute(
        "UPDATE changed_path_observer_segments
         SET folded_end_offset=durable_end_offset,updated_at=?1
         WHERE scope_id=?2 AND epoch=1 AND segment_id=?3",
        params![now_ts(), scope_text, cut.segment_id],
    )?;
    conn.execute(
        "UPDATE changed_path_scopes
         SET trust_state='trusted',trust_reason='authenticated_clean_checkpoint',
             durable_offset=?1,folded_offset=?1,observer_owner_token=?2,
             observer_heartbeat_at=?3,updated_at=?3
         WHERE scope_id=?4 AND epoch=1",
        params![cut.durable_end_offset, owner_text, now_ts(), scope_text],
    )?;
    drop(conn);
    let sparse_selection_fingerprint =
        super::workdir::actual_sparse_selection_fingerprint_read_only(&workdir)
            .map_err(|error| Error::Corrupt(format!("fixture sparse fingerprint: {error}")))?;
    super::workdir::write_materialized_lane_marker_v2_for_test(
        &workdir,
        &super::workdir::MaterializedLaneMarkerV2 {
            version: super::workdir::MATERIALIZED_LANE_MARKER_VERSION,
            scope_id,
            filesystem_identity,
            ref_name,
            ref_generation: u64::try_from(ref_generation)
                .map_err(|_| Error::Corrupt("negative fixture ref generation".into()))?,
            root_id: ObjectId(head_root),
            policy_fingerprint,
            epoch: 1,
            provider_cut: crate::db::change_ledger::EvidenceCut {
                source: crate::db::change_ledger::EvidenceSource::Observer,
                sequence: cut.last_sequence,
                durable_offset: cut.durable_end_offset,
                folded_offset: cut.durable_end_offset,
            },
            provider_segment_id: cut.segment_id,
            sparse_selection_fingerprint,
        },
    )
    .map_err(|error| Error::Corrupt(format!("fixture marker write: {error}")))?;
    let conn = Connection::open(&database_path)?;
    let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
    if busy != 0 {
        return Err(Error::WorkspaceLocked(
            "authenticated schema18 fixture checkpoint remained busy".into(),
        ));
    }
    Ok(())
}

fn schema_v19_backfill_now() -> i64 {
    #[cfg(any(test, debug_assertions))]
    if let Some(now) =
        SCHEMA_V19_BACKFILL_TIMES.with(|installed| installed.borrow_mut().pop_front())
    {
        return now;
    }
    now_ts()
}

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

#[derive(Clone, Debug, serde::Serialize)]
struct CanonicalLegacyLaneMetadata {
    requested_workdir_mode: String,
    workdir_mode: String,
    workdir_backend: String,
    materialization: Option<MaterializationReport>,
    sparse_paths: Vec<String>,
    include_neighbors: bool,
    transparent_cow_available: bool,
}

fn parse_legacy_lane_metadata(
    metadata_json: Option<&str>,
) -> std::result::Result<CanonicalLegacyLaneMetadata, String> {
    let encoded = metadata_json.ok_or_else(|| "lane metadata_json is missing".to_string())?;
    let value: serde_json::Value = serde_json::from_str(encoded)
        .map_err(|error| format!("lane metadata_json is malformed: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "lane metadata_json must be an object".to_string())?;
    let required_text = |field: &str| {
        object
            .get(field)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| format!("lane metadata `{field}` must be a string"))
    };
    let requested_workdir_mode =
        LaneWorkdirMode::parse(required_text("requested_workdir_mode")?)
            .ok_or_else(|| "lane metadata `requested_workdir_mode` is invalid".to_string())?;
    let workdir_mode = LaneWorkdirMode::parse(required_text("workdir_mode")?)
        .ok_or_else(|| "lane metadata `workdir_mode` is invalid".to_string())?;
    let workdir_backend = serde_json::from_value::<WorkdirBackend>(
        object
            .get("workdir_backend")
            .cloned()
            .ok_or_else(|| "lane metadata `workdir_backend` is missing".to_string())?,
    )
    .map_err(|_| "lane metadata `workdir_backend` is invalid".to_string())?;
    let materialization = object
        .get("materialization")
        .filter(|value| !value.is_null())
        .cloned()
        .map(serde_json::from_value::<MaterializationReport>)
        .transpose()
        .map_err(|error| format!("lane metadata `materialization` is incomplete: {error}"))?;
    let sparse_paths = object
        .get("sparse_paths")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| "lane metadata `sparse_paths` must be an array".to_string())?;
    let mut sparse_paths = sparse_paths
        .iter()
        .enumerate()
        .map(|(index, value)| {
            let path = value
                .as_str()
                .ok_or_else(|| format!("lane metadata `sparse_paths[{index}]` must be a string"))?;
            normalize_relative_path(path).map_err(|error| {
                format!("lane metadata `sparse_paths[{index}]` is invalid: {error}")
            })
        })
        .collect::<std::result::Result<Vec<_>, _>>()?;
    sparse_paths.sort();
    sparse_paths.dedup();
    let include_neighbors = object
        .get("include_neighbors")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| "lane metadata `include_neighbors` must be a boolean".to_string())?;
    let transparent_cow_available = object
        .get("transparent_cow_available")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| "lane metadata `transparent_cow_available` must be a boolean".to_string())?;
    Ok(CanonicalLegacyLaneMetadata {
        requested_workdir_mode: requested_workdir_mode.as_str().to_string(),
        workdir_mode: workdir_mode.as_str().to_string(),
        workdir_backend: workdir_backend.as_str().to_string(),
        materialization,
        sparse_paths,
        include_neighbors,
        transparent_cow_available,
    })
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
                 WHERE lane_name=?1 OR lane_id=?1
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
        let metadata = parse_legacy_lane_metadata(metadata_json.as_deref());
        let fingerprint_metadata = match &metadata {
            Ok(metadata) => serde_json::to_value(metadata)?,
            Err(_) => {
                let mut digest = Sha256::new();
                digest.update(b"trail-invalid-legacy-lane-metadata-v1\0");
                match metadata_json.as_deref() {
                    Some(encoded) => digest.update(encoded.as_bytes()),
                    None => digest.update(b"missing"),
                }
                serde_json::json!({
                    "invalid_metadata_sha256": hex::encode(digest.finalize()),
                })
            }
        };
        let fingerprint_value = serde_json::json!({
            "version": "legacy_lane_initialization_v1",
            "lane_name": lane_name,
            "base_change": base_change,
            "base_root": base_root,
            "workdir": workdir,
            "metadata": fingerprint_metadata,
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
        let now = schema_v19_backfill_now();
        let failure = match &metadata {
            Ok(metadata) => {
                backfilled_lane_failed_invariant(tx, &lane_id, workdir.as_deref(), metadata, now)?
            }
            Err(error) => Some(error.clone()),
        };
        let phase = backfilled_lane_phase(failure.as_deref());
        let phase_text = match phase {
            LaneInitializationPhase::ObserverReady => "observer_ready",
            LaneInitializationPhase::RepairRequired => "repair_required",
            _ => unreachable!("legacy backfill only writes terminal phases"),
        };
        let materialization_json = metadata
            .as_ref()
            .ok()
            .and_then(|metadata| metadata.materialization.as_ref())
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

fn backfilled_lane_phase(failed_invariant: Option<&str>) -> LaneInitializationPhase {
    if failed_invariant.is_none() {
        LaneInitializationPhase::ObserverReady
    } else {
        LaneInitializationPhase::RepairRequired
    }
}

#[derive(Debug)]
struct LegacyObserverEvidence {
    scope_id: String,
    epoch: i64,
    filesystem_identity: String,
    policy_fingerprint: String,
    durable_offset: i64,
    folded_offset: i64,
    max_observer_log_bytes: i64,
    max_segment_bytes: i64,
    max_unfolded_tail_records: i64,
    owner_token: String,
    ref_name: String,
    ref_generation: i64,
    root_id: String,
}

fn authenticated_legacy_observer_evidence(
    tx: &rusqlite::Transaction<'_>,
    lane_id: &str,
    workdir: &str,
    now: i64,
) -> Result<bool> {
    let evidence = tx
        .query_row(
            "SELECT scope.scope_id,scope.epoch,scope.filesystem_identity,
                    scope.policy_fingerprint,scope.durable_offset,scope.folded_offset,
                    scope.max_observer_log_bytes,scope.max_segment_bytes,
                    scope.max_unfolded_tail_records,owner.owner_token,
                    ref.name,ref.generation,branch.head_root
             FROM changed_path_scopes scope
             JOIN lane_branches branch ON branch.lane_id=scope.owner_id
             JOIN refs ref ON ref.name=branch.ref_name
             JOIN changed_path_observer_owners owner
               ON owner.scope_id=scope.scope_id AND owner.epoch=scope.epoch
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
               AND scope.observer_error_state IS NULL AND scope.observer_error_at IS NULL
               AND owner.lease_state='active' AND owner.error_state IS NULL
               AND owner.error_at IS NULL AND owner.expires_at>?3
               AND owner.fence_nonce IS NOT NULL AND length(owner.fence_nonce)>=16
               AND owner.provider_id=scope.provider_id
               AND owner.provider_identity=scope.provider_identity
               AND owner.provider_id=owner.provider_identity
               AND scope.observer_owner_token=owner.owner_token
               AND EXISTS(
                   SELECT 1 FROM changed_path_policy_dependencies dependency
                   WHERE dependency.scope_id=scope.scope_id
                     AND dependency.generation=scope.policy_dependency_generation)
               AND NOT EXISTS(
                   SELECT 1 FROM changed_path_observer_segments segment
                   WHERE segment.scope_id=scope.scope_id AND segment.epoch=scope.epoch
                     AND (segment.owner_token<>owner.owner_token
                          OR segment.provider_id<>owner.provider_id
                          OR segment.state NOT IN ('open','sealed')))
               AND scope.retired_at IS NULL",
            params![lane_id, workdir, now],
            |row| {
                Ok(LegacyObserverEvidence {
                    scope_id: row.get(0)?,
                    epoch: row.get(1)?,
                    filesystem_identity: row.get(2)?,
                    policy_fingerprint: row.get(3)?,
                    durable_offset: row.get(4)?,
                    folded_offset: row.get(5)?,
                    max_observer_log_bytes: row.get(6)?,
                    max_segment_bytes: row.get(7)?,
                    max_unfolded_tail_records: row.get(8)?,
                    owner_token: row.get(9)?,
                    ref_name: row.get(10)?,
                    ref_generation: row.get(11)?,
                    root_id: row.get(12)?,
                })
            },
        )
        .optional()?;
    let Some(evidence) = evidence else {
        return Ok(false);
    };
    let Some(scope_id) = decode_canonical_hex_32(&evidence.scope_id) else {
        return Ok(false);
    };
    let Some(owner_token) = decode_canonical_hex_32(&evidence.owner_token) else {
        return Ok(false);
    };
    let Some(policy_fingerprint) = decode_canonical_hex_32(&evidence.policy_fingerprint) else {
        return Ok(false);
    };
    let workdir = Path::new(workdir);
    let actual_root_identity = match super::workdir::materialized_lane_root_identity(workdir) {
        Ok(identity) => identity,
        Err(_) => return Ok(false),
    };
    if hex::encode(&actual_root_identity) != evidence.filesystem_identity {
        return Ok(false);
    }
    let marker = match super::workdir::read_materialized_lane_marker_v2(workdir) {
        Ok(Some(marker)) => marker,
        Ok(None) | Err(_) => return Ok(false),
    };
    let sparse_fingerprint =
        match super::workdir::actual_sparse_selection_fingerprint_read_only(workdir) {
            Ok(fingerprint) => fingerprint,
            Err(_) => return Ok(false),
        };
    let epoch = match u64::try_from(evidence.epoch) {
        Ok(epoch) => epoch,
        Err(_) => return Ok(false),
    };
    let ref_generation = match u64::try_from(evidence.ref_generation) {
        Ok(generation) => generation,
        Err(_) => return Ok(false),
    };
    let durable_offset = match u64::try_from(evidence.durable_offset) {
        Ok(offset) => offset,
        Err(_) => return Ok(false),
    };
    if evidence.folded_offset != evidence.durable_offset
        || marker.scope_id != crate::db::change_ledger::ScopeId(scope_id)
        || marker.filesystem_identity != actual_root_identity
        || marker.ref_name != evidence.ref_name
        || marker.ref_generation != ref_generation
        || marker.root_id.0 != evidence.root_id
        || marker.policy_fingerprint != policy_fingerprint
        || marker.epoch != epoch
        || marker.provider_cut.source != crate::db::change_ledger::EvidenceSource::Observer
        || marker.provider_cut.durable_offset != durable_offset
        || marker.provider_cut.folded_offset != durable_offset
        || marker.sparse_selection_fingerprint != sparse_fingerprint
        || marker.provider_segment_id.is_empty()
    {
        return Ok(false);
    }
    let database_path: String = tx.query_row(
        "SELECT file FROM pragma_database_list WHERE name='main'",
        [],
        |row| row.get(0),
    )?;
    let workspace_db_dir =
        crate::db::change_ledger::workspace_db_dir_for_database(Path::new(&database_path))?;
    let segment_directory = workspace_db_dir
        .join("observer-segments")
        .join(&evidence.scope_id);
    let segment_directory =
        match crate::db::change_ledger::secure_fs::SecureDirectory::open_absolute(
            &segment_directory,
        ) {
            Ok(directory) => directory,
            Err(_) => return Ok(false),
        };
    let limits = match (
        u64::try_from(evidence.max_observer_log_bytes),
        u64::try_from(evidence.max_segment_bytes),
        usize::try_from(evidence.max_unfolded_tail_records),
    ) {
        (Ok(max_log_bytes), Ok(max_segment_bytes), Ok(max_unfolded_tail_records)) => {
            crate::db::change_ledger::PersistedLogLimits {
                max_log_bytes,
                max_segment_bytes,
                max_unfolded_tail_records,
            }
        }
        _ => return Ok(false),
    };
    let recovered = match crate::db::change_ledger::recover_segments_from_connection(
        tx,
        &segment_directory,
        &crate::db::change_ledger::RecoveryScope {
            scope_id: crate::db::change_ledger::ScopeId(scope_id),
            epoch,
            owner_token,
        },
        limits,
    ) {
        Ok(recovered) => recovered,
        Err(_) => return Ok(false),
    };
    Ok(!recovered.requires_reconciliation
        && recovered.durable_end == durable_offset
        && recovered.segments.iter().any(|segment| {
            segment.segment_id == marker.provider_segment_id
                && segment.folded_end_offset == durable_offset
                && segment.durable_end_offset >= durable_offset
                && segment.first_sequence <= marker.provider_cut.sequence
                && segment.last_sequence >= marker.provider_cut.sequence
        }))
}

fn decode_canonical_hex_32(encoded: &str) -> Option<[u8; 32]> {
    if encoded.len() != 64
        || !encoded
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return None;
    }
    hex::decode(encoded).ok()?.try_into().ok()
}

fn backfilled_lane_failed_invariant(
    tx: &rusqlite::Transaction<'_>,
    lane_id: &str,
    workdir: Option<&str>,
    metadata: &CanonicalLegacyLaneMetadata,
    now: i64,
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
        if metadata.materialization.is_none() {
            return Ok(Some(
                "materialized lane metadata is missing or incomplete".into(),
            ));
        }
        if !authenticated_legacy_observer_evidence(tx, lane_id, workdir.unwrap(), now)? {
            return Ok(Some(
                "materialized lane authenticated observer/filesystem/policy evidence is missing or inconsistent".into(),
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
