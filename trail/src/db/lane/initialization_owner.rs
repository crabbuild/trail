use super::*;
use crate::db::lane::initialization::{
    insert_lane_initialization_reservation, lane_initialization_record, LaneInitializationRecord,
    ResolvedLaneSpawnRequest,
};
use crate::db::util::{
    current_process_start_token, process_start_token_match, ProcessIdentityMatch,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LaneInitializationFence {
    pub(crate) owner_token: String,
    pub(crate) owner_generation: i64,
}

#[derive(Clone, Debug)]
pub(crate) enum LaneInitializationClaim {
    Owned {
        record: LaneInitializationRecord,
        fence: LaneInitializationFence,
        resumed: bool,
    },
    Contended {
        record: LaneInitializationRecord,
        owner_pid: u32,
    },
    Terminal(LaneInitializationRecord),
}

#[derive(Clone, Debug)]
pub(crate) enum LaneInitializationRepairClaim {
    Owned {
        record: LaneInitializationRecord,
        fence: LaneInitializationFence,
    },
    Contended {
        record: LaneInitializationRecord,
        owner_pid: u32,
    },
    Terminal(LaneInitializationRecord),
}

#[derive(Clone, Debug)]
struct LaneInitializationOwner {
    fence: LaneInitializationFence,
    owner_pid: u32,
    owner_process_start_identity: String,
}

fn lane_initialization_owner(
    conn: &Connection,
    initialization_id: &str,
) -> Result<Option<LaneInitializationOwner>> {
    conn.query_row(
        "SELECT owner_token,owner_generation,owner_pid,owner_process_start_identity
         FROM lane_initialization_owners WHERE initialization_id=?1",
        [initialization_id],
        |row| {
            Ok(LaneInitializationOwner {
                fence: LaneInitializationFence {
                    owner_token: row.get(0)?,
                    owner_generation: row.get(1)?,
                },
                owner_pid: row.get(2)?,
                owner_process_start_identity: row.get(3)?,
            })
        },
    )
    .optional()
    .map_err(Into::into)
}

fn new_owner_token() -> Result<String> {
    let mut token = [0_u8; 32];
    getrandom::getrandom(&mut token)
        .map_err(|error| Error::Io(std::io::Error::other(error.to_string())))?;
    Ok(hex::encode(token))
}

fn terminal_phase(phase: LaneInitializationPhase) -> bool {
    matches!(
        phase,
        LaneInitializationPhase::ObserverReady | LaneInitializationPhase::RepairRequired
    )
}

fn validate_matching_request(
    record: &LaneInitializationRecord,
    request: &ResolvedLaneSpawnRequest,
) -> Result<()> {
    if record.request_fingerprint == request.request_fingerprint {
        return Ok(());
    }
    Err(Error::LaneInitializationConflict {
        lane: request.lane_name.clone(),
        existing_fingerprint: record.request_fingerprint.clone(),
        requested_fingerprint: request.request_fingerprint.clone(),
    })
}

#[cfg(any(test, debug_assertions))]
thread_local! {
    static PROCESS_LIVENESS_OVERRIDES: std::cell::RefCell<
        std::collections::HashMap<(u32, String), ProcessIdentityMatch>
    > = std::cell::RefCell::new(std::collections::HashMap::new());
}

#[cfg(any(test, debug_assertions))]
pub(crate) fn clear_process_liveness_overrides() {
    PROCESS_LIVENESS_OVERRIDES.with(|overrides| overrides.borrow_mut().clear());
}

#[cfg(test)]
fn install_process_liveness_override(pid: u32, start_identity: &str, live: bool) {
    let result = if live {
        ProcessIdentityMatch::Match
    } else {
        ProcessIdentityMatch::DeadOrMismatch
    };
    PROCESS_LIVENESS_OVERRIDES.with(|overrides| {
        overrides
            .borrow_mut()
            .insert((pid, start_identity.to_string()), result);
    });
}

#[cfg(any(test, debug_assertions))]
pub(crate) fn install_process_liveness_unknown_override(pid: u32, start_identity: &str) {
    PROCESS_LIVENESS_OVERRIDES.with(|overrides| {
        overrides.borrow_mut().insert(
            (pid, start_identity.to_string()),
            ProcessIdentityMatch::Unknown,
        );
    });
}

fn owner_process_match(pid: u32, start_identity: &str) -> ProcessIdentityMatch {
    #[cfg(any(test, debug_assertions))]
    if let Some(result) = PROCESS_LIVENESS_OVERRIDES.with(|overrides| {
        overrides
            .borrow()
            .get(&(pid, start_identity.to_string()))
            .copied()
    }) {
        return result;
    }
    process_start_token_match(pid, start_identity)
}

#[cfg(debug_assertions)]
thread_local! {
    static STEAL_OWNER_ON_NEXT_HEARTBEAT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

#[cfg(debug_assertions)]
pub(crate) fn steal_owner_on_next_heartbeat_for_current_thread() {
    STEAL_OWNER_ON_NEXT_HEARTBEAT.with(|enabled| enabled.set(true));
}

pub(crate) fn claim_lane_initialization_owner(
    db: &mut Trail,
    request: &ResolvedLaneSpawnRequest,
) -> Result<LaneInitializationClaim> {
    loop {
        let Some(record) = lane_initialization_record(&db.conn, &request.lane_name)? else {
            let owner_token = new_owner_token()?;
            let owner_pid = std::process::id();
            let owner_process_start_identity = current_process_start_token();
            let tx = db
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            if lane_initialization_record(&tx, &request.lane_name)?.is_some() {
                tx.commit()?;
                continue;
            }
            let record = insert_lane_initialization_reservation(&tx, request)?;
            let now = now_ts();
            tx.execute(
                "INSERT INTO lane_initialization_owners(
                     initialization_id,owner_token,owner_generation,owner_pid,
                     owner_process_start_identity,acquired_at,heartbeat_at)
                 VALUES(?1,?2,1,?3,?4,?5,?5)",
                params![
                    request.initialization_id,
                    owner_token,
                    owner_pid,
                    owner_process_start_identity,
                    now,
                ],
            )?;
            tx.commit()?;
            return Ok(LaneInitializationClaim::Owned {
                record,
                fence: LaneInitializationFence {
                    owner_token,
                    owner_generation: 1,
                },
                resumed: false,
            });
        };

        validate_matching_request(&record, request)?;
        if terminal_phase(record.phase) {
            return Ok(LaneInitializationClaim::Terminal(record));
        }

        let Some(owner) = lane_initialization_owner(&db.conn, &record.initialization_id)? else {
            let owner_token = new_owner_token()?;
            let owner_pid = std::process::id();
            let owner_process_start_identity = current_process_start_token();
            let now = now_ts();
            let tx = db
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let Some(current) = lane_initialization_record(&tx, &request.lane_name)? else {
                tx.commit()?;
                continue;
            };
            validate_matching_request(&current, request)?;
            if terminal_phase(current.phase) {
                tx.commit()?;
                return Ok(LaneInitializationClaim::Terminal(current));
            }
            let changed = tx.execute(
                "INSERT INTO lane_initialization_owners(
                     initialization_id,owner_token,owner_generation,owner_pid,
                     owner_process_start_identity,acquired_at,heartbeat_at)
                 VALUES(?1,?2,1,?3,?4,?5,?5)
                 ON CONFLICT(initialization_id) DO NOTHING",
                params![
                    current.initialization_id,
                    owner_token,
                    owner_pid,
                    owner_process_start_identity,
                    now,
                ],
            )?;
            tx.commit()?;
            if changed == 0 {
                continue;
            }
            return Ok(LaneInitializationClaim::Owned {
                record: current,
                fence: LaneInitializationFence {
                    owner_token,
                    owner_generation: 1,
                },
                resumed: true,
            });
        };

        if owner_process_match(owner.owner_pid, &owner.owner_process_start_identity)
            != ProcessIdentityMatch::DeadOrMismatch
        {
            return Ok(LaneInitializationClaim::Contended {
                record,
                owner_pid: owner.owner_pid,
            });
        }

        let owner_token = new_owner_token()?;
        let owner_pid = std::process::id();
        let owner_process_start_identity = current_process_start_token();
        let now = now_ts();
        let tx = db
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let Some(current) = lane_initialization_record(&tx, &request.lane_name)? else {
            tx.commit()?;
            continue;
        };
        validate_matching_request(&current, request)?;
        if terminal_phase(current.phase) {
            tx.commit()?;
            return Ok(LaneInitializationClaim::Terminal(current));
        }
        let changed = tx.execute(
            "UPDATE lane_initialization_owners
             SET owner_token=?1, owner_generation=owner_generation+1,
                 owner_pid=?2, owner_process_start_identity=?3,
                 acquired_at=?4, heartbeat_at=?4
             WHERE initialization_id=?5
               AND owner_token=?6 AND owner_generation=?7
               AND owner_pid=?8 AND owner_process_start_identity=?9",
            params![
                owner_token,
                owner_pid,
                owner_process_start_identity,
                now,
                current.initialization_id,
                owner.fence.owner_token,
                owner.fence.owner_generation,
                owner.owner_pid,
                owner.owner_process_start_identity,
            ],
        )?;
        tx.commit()?;
        if changed == 0 {
            continue;
        }
        return Ok(LaneInitializationClaim::Owned {
            record: current,
            fence: LaneInitializationFence {
                owner_token,
                owner_generation: owner.fence.owner_generation + 1,
            },
            resumed: true,
        });
    }
}

pub(crate) fn claim_lane_initialization_repair(
    db: &mut Trail,
    lane: &str,
) -> Result<LaneInitializationRepairClaim> {
    loop {
        let record = lane_initialization_record(&db.conn, lane)?
            .ok_or_else(|| Error::Corrupt(format!("lane `{lane}` has no initialization row")))?;
        if record.phase == LaneInitializationPhase::ObserverReady {
            return Ok(LaneInitializationRepairClaim::Terminal(record));
        }
        if !matches!(
            record.phase,
            LaneInitializationPhase::Associated | LaneInitializationPhase::RepairRequired
        ) {
            return Err(Error::Corrupt(format!(
                "lane initialization `{}` is {:?}, expected associated or repair_required",
                record.initialization_id, record.phase
            )));
        }

        let existing_owner = lane_initialization_owner(&db.conn, &record.initialization_id)?;
        if let Some(owner) = existing_owner {
            let owner_match =
                owner_process_match(owner.owner_pid, &owner.owner_process_start_identity);
            if owner_match != ProcessIdentityMatch::DeadOrMismatch {
                return Ok(LaneInitializationRepairClaim::Contended {
                    record,
                    owner_pid: owner.owner_pid,
                });
            }

            let owner_token = new_owner_token()?;
            let owner_pid = std::process::id();
            let owner_process_start_identity = current_process_start_token();
            let now = now_ts();
            let tx = db
                .conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
            let Some(current) = lane_initialization_record(&tx, lane)? else {
                tx.commit()?;
                continue;
            };
            if current.phase == LaneInitializationPhase::ObserverReady {
                tx.commit()?;
                return Ok(LaneInitializationRepairClaim::Terminal(current));
            }
            if !matches!(
                current.phase,
                LaneInitializationPhase::Associated | LaneInitializationPhase::RepairRequired
            ) {
                tx.commit()?;
                continue;
            }
            if current.phase == LaneInitializationPhase::Associated {
                let transitioned = tx.execute(
                    "UPDATE lane_initializations
                     SET phase='repair_required',
                         repair_command='trail lane repair-initialization ' || lane_name,
                         updated_at=?1
                     WHERE initialization_id=?2 AND phase='associated'
                       AND EXISTS(
                         SELECT 1 FROM lane_initialization_owners owner
                         WHERE owner.initialization_id=lane_initializations.initialization_id
                           AND owner.owner_token=?3 AND owner.owner_generation=?4
                           AND owner.owner_pid=?5
                           AND owner.owner_process_start_identity=?6)",
                    params![
                        now,
                        current.initialization_id,
                        owner.fence.owner_token,
                        owner.fence.owner_generation,
                        owner.owner_pid,
                        owner.owner_process_start_identity,
                    ],
                )?;
                if transitioned == 0 {
                    drop(tx);
                    continue;
                }
            }
            let changed = tx.execute(
                "UPDATE lane_initialization_owners
                 SET owner_token=?1, owner_generation=owner_generation+1,
                     owner_pid=?2, owner_process_start_identity=?3,
                     acquired_at=?4, heartbeat_at=?4
                 WHERE initialization_id=?5
                   AND owner_token=?6 AND owner_generation=?7
                   AND owner_pid=?8 AND owner_process_start_identity=?9",
                params![
                    owner_token,
                    owner_pid,
                    owner_process_start_identity,
                    now,
                    current.initialization_id,
                    owner.fence.owner_token,
                    owner.fence.owner_generation,
                    owner.owner_pid,
                    owner.owner_process_start_identity,
                ],
            )?;
            if changed == 0 {
                drop(tx);
                continue;
            }
            let claimed =
                lane_initialization_record(&tx, &current.initialization_id)?.ok_or_else(|| {
                    Error::Corrupt(format!(
                        "lane initialization `{}` disappeared during repair takeover",
                        current.initialization_id
                    ))
                })?;
            tx.commit()?;
            return Ok(LaneInitializationRepairClaim::Owned {
                record: claimed,
                fence: LaneInitializationFence {
                    owner_token,
                    owner_generation: owner.fence.owner_generation + 1,
                },
            });
        }

        let owner_token = new_owner_token()?;
        let owner_pid = std::process::id();
        let owner_process_start_identity = current_process_start_token();
        let now = now_ts();
        let tx = db
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let Some(current) = lane_initialization_record(&tx, lane)? else {
            tx.commit()?;
            continue;
        };
        if current.phase == LaneInitializationPhase::ObserverReady {
            tx.commit()?;
            return Ok(LaneInitializationRepairClaim::Terminal(current));
        }
        if !matches!(
            current.phase,
            LaneInitializationPhase::Associated | LaneInitializationPhase::RepairRequired
        ) {
            return Err(Error::Corrupt(format!(
                "lane initialization `{}` changed to {:?} during repair claim",
                current.initialization_id, current.phase
            )));
        }
        if current.phase == LaneInitializationPhase::Associated {
            let changed = tx.execute(
                "UPDATE lane_initializations
                 SET phase='repair_required',
                     repair_command='trail lane repair-initialization ' || lane_name,
                     updated_at=?1
                 WHERE initialization_id=?2 AND phase='associated'
                   AND NOT EXISTS(
                     SELECT 1 FROM lane_initialization_owners owner
                     WHERE owner.initialization_id=lane_initializations.initialization_id)",
                params![now, current.initialization_id],
            )?;
            if changed == 0 {
                tx.commit()?;
                continue;
            }
        }
        let inserted = tx.execute(
            "INSERT INTO lane_initialization_owners(
                 initialization_id,owner_token,owner_generation,owner_pid,
                 owner_process_start_identity,acquired_at,heartbeat_at)
             VALUES(?1,?2,1,?3,?4,?5,?5)
             ON CONFLICT(initialization_id) DO NOTHING",
            params![
                current.initialization_id,
                owner_token,
                owner_pid,
                owner_process_start_identity,
                now,
            ],
        )?;
        if inserted == 0 {
            tx.commit()?;
            continue;
        }
        let claimed =
            lane_initialization_record(&tx, &current.initialization_id)?.ok_or_else(|| {
                Error::Corrupt(format!(
                    "lane initialization `{}` disappeared during repair claim",
                    current.initialization_id
                ))
            })?;
        tx.commit()?;
        return Ok(LaneInitializationRepairClaim::Owned {
            record: claimed,
            fence: LaneInitializationFence {
                owner_token,
                owner_generation: 1,
            },
        });
    }
}

pub(crate) fn heartbeat_lane_initialization_owner(
    conn: &Connection,
    initialization_id: &str,
    fence: &LaneInitializationFence,
) -> Result<()> {
    #[cfg(debug_assertions)]
    if STEAL_OWNER_ON_NEXT_HEARTBEAT.with(|enabled| enabled.replace(false)) {
        conn.execute(
            "UPDATE lane_initialization_owners
             SET owner_token=?1,owner_generation=owner_generation+1,
                 owner_pid=?2,owner_process_start_identity='dead-test-owner'
             WHERE initialization_id=?3 AND owner_token=?4 AND owner_generation=?5",
            params![
                "55".repeat(32),
                u32::MAX,
                initialization_id,
                fence.owner_token,
                fence.owner_generation,
            ],
        )?;
    }
    let changed = conn.execute(
        "UPDATE lane_initialization_owners SET heartbeat_at=?1
         WHERE initialization_id=?2 AND owner_token=?3 AND owner_generation=?4",
        params![
            now_ts(),
            initialization_id,
            fence.owner_token,
            fence.owner_generation,
        ],
    )?;
    if changed == 1 {
        return Ok(());
    }
    Err(Error::LaneInitializationOwnershipLost {
        initialization_id: initialization_id.to_string(),
    })
}

pub(crate) fn release_lane_initialization_owner(
    conn: &Connection,
    initialization_id: &str,
    fence: &LaneInitializationFence,
) -> Result<bool> {
    Ok(conn.execute(
        "DELETE FROM lane_initialization_owners
         WHERE initialization_id=?1 AND owner_token=?2 AND owner_generation=?3",
        params![initialization_id, fence.owner_token, fence.owner_generation,],
    )? == 1)
}

pub(crate) fn owner_fence_matches(
    conn: &Connection,
    initialization_id: &str,
    fence: &LaneInitializationFence,
) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(
             SELECT 1 FROM lane_initialization_owners
             WHERE initialization_id=?1 AND owner_token=?2 AND owner_generation=?3)",
        params![initialization_id, fence.owner_token, fence.owner_generation,],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::lane::initialization::ResolvedLaneSpawnRequest;

    struct OwnerFixture {
        db: Trail,
        request: ResolvedLaneSpawnRequest,
    }

    impl OwnerFixture {
        fn new() -> Self {
            clear_process_liveness_overrides();
            let workspace = tempfile::tempdir().unwrap();
            Trail::init(workspace.path(), "main", InitImportMode::Empty, false).unwrap();
            let mut db = Trail::open(workspace.path()).unwrap();
            db.conn = Connection::open_in_memory().unwrap();
            db.create_schema_v20().unwrap();
            let request = ResolvedLaneSpawnRequest::new(
                "workspace-test",
                "lane-a",
                "lane_test".to_string(),
                "refs/branches/main".to_string(),
                ChangeId("change_test".to_string()),
                ObjectId("root_test".to_string()),
                ObjectId("operation_test".to_string()),
                LaneWorkdirMode::Virtual,
                None,
                Vec::new(),
                false,
                None,
                None,
            )
            .unwrap();
            Self { db, request }
        }

        fn claim(&mut self, lane: &str) -> LaneInitializationClaim {
            assert_eq!(lane, self.request.lane_name);
            let claim = claim_lane_initialization_owner(&mut self.db, &self.request).unwrap();
            if let LaneInitializationClaim::Owned { fence, .. } = &claim {
                let (pid, start_identity): (u32, String) = self
                    .db
                    .conn
                    .query_row(
                        "SELECT owner_pid,owner_process_start_identity
                         FROM lane_initialization_owners WHERE initialization_id=?1",
                        [&self.request.initialization_id],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .unwrap();
                install_process_liveness_override(pid, &start_identity, true);
                assert!(
                    owner_fence_matches(&self.db.conn, &self.request.initialization_id, fence)
                        .unwrap()
                );
            }
            claim
        }

        fn age_heartbeat(&self, lane: &str, heartbeat: i64) {
            assert_eq!(lane, self.request.lane_name);
            self.db
                .conn
                .execute(
                    "UPDATE lane_initialization_owners SET heartbeat_at=?1
                     WHERE initialization_id=?2",
                    params![heartbeat, self.request.initialization_id],
                )
                .unwrap();
        }

        fn mark_owner_dead(&self, lane: &str) {
            assert_eq!(lane, self.request.lane_name);
            let (pid, start_identity): (u32, String) = self
                .db
                .conn
                .query_row(
                    "SELECT owner_pid,owner_process_start_identity
                     FROM lane_initialization_owners WHERE initialization_id=?1",
                    [&self.request.initialization_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            install_process_liveness_override(pid, &start_identity, false);
        }

        fn mark_owner_unknown(&self, lane: &str) {
            assert_eq!(lane, self.request.lane_name);
            let (pid, start_identity): (u32, String) = self
                .db
                .conn
                .query_row(
                    "SELECT owner_pid,owner_process_start_identity
                     FROM lane_initialization_owners WHERE initialization_id=?1",
                    [&self.request.initialization_id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .unwrap();
            install_process_liveness_unknown_override(pid, &start_identity);
        }

        fn install_owner(&self, pid: u32, start_identity: &str) {
            self.insert_initialization("reserved");
            self.db
                .conn
                .execute(
                    "INSERT INTO lane_initialization_owners(
                         initialization_id,owner_token,owner_generation,owner_pid,
                         owner_process_start_identity,acquired_at,heartbeat_at)
                     VALUES(?1,?2,1,?3,?4,1,1)",
                    params![
                        self.request.initialization_id,
                        hex::encode([0x11_u8; 32]),
                        pid,
                        start_identity,
                    ],
                )
                .unwrap();
            install_process_liveness_override(pid, start_identity, false);
        }

        fn install_repair_owner(&self, pid: u32, start_identity: &str, heartbeat_at: i64) {
            self.install_active_owner("repair_required", pid, start_identity, heartbeat_at);
        }

        fn install_associated_owner(&self, pid: u32, start_identity: &str, heartbeat_at: i64) {
            self.install_active_owner("associated", pid, start_identity, heartbeat_at);
        }

        fn install_active_owner(
            &self,
            phase: &str,
            pid: u32,
            start_identity: &str,
            heartbeat_at: i64,
        ) {
            self.insert_initialization(phase);
            self.db
                .conn
                .execute(
                    "INSERT INTO lane_initialization_owners(
                         initialization_id,owner_token,owner_generation,owner_pid,
                         owner_process_start_identity,acquired_at,heartbeat_at)
                     VALUES(?1,?2,1,?3,?4,1,?5)",
                    params![
                        self.request.initialization_id,
                        hex::encode([0x22_u8; 32]),
                        pid,
                        start_identity,
                        heartbeat_at,
                    ],
                )
                .unwrap();
        }

        fn claim_repair(&mut self) -> LaneInitializationRepairClaim {
            claim_lane_initialization_repair(&mut self.db, &self.request.lane_name).unwrap()
        }

        fn install_terminal(&self, lane: &str, phase: LaneInitializationPhase) {
            assert_eq!(lane, self.request.lane_name);
            let phase = match phase {
                LaneInitializationPhase::ObserverReady => "observer_ready",
                LaneInitializationPhase::RepairRequired => "repair_required",
                other => panic!("{other:?} is not terminal"),
            };
            self.insert_initialization(phase);
        }

        fn owner_count(&self, lane: &str) -> i64 {
            assert_eq!(lane, self.request.lane_name);
            self.db
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM lane_initialization_owners
                     WHERE initialization_id=?1",
                    [&self.request.initialization_id],
                    |row| row.get(0),
                )
                .unwrap()
        }

        fn stored_fence(&self, lane: &str) -> LaneInitializationFence {
            assert_eq!(lane, self.request.lane_name);
            self.db
                .conn
                .query_row(
                    "SELECT owner_token,owner_generation FROM lane_initialization_owners
                     WHERE initialization_id=?1",
                    [&self.request.initialization_id],
                    |row| {
                        Ok(LaneInitializationFence {
                            owner_token: row.get(0)?,
                            owner_generation: row.get(1)?,
                        })
                    },
                )
                .unwrap()
        }

        fn owner_fence_matches(&self, lane: &str, fence: &LaneInitializationFence) -> bool {
            assert_eq!(lane, self.request.lane_name);
            owner_fence_matches(&self.db.conn, &self.request.initialization_id, fence).unwrap()
        }

        fn insert_initialization(&self, phase: &str) {
            self.db
                .conn
                .execute(
                    "INSERT INTO lane_initializations(
                         initialization_id,lane_name,lane_id,request_fingerprint,operation_id,
                         phase,workdir,materialization_json,last_error_code,last_error_message,
                         repair_command,created_at,updated_at)
                     VALUES(?1,?2,?3,?4,?5,?6,NULL,NULL,NULL,NULL,NULL,1,1)",
                    params![
                        self.request.initialization_id,
                        self.request.lane_name,
                        self.request.lane_id,
                        self.request.request_fingerprint,
                        self.request.source_operation.0,
                        phase,
                    ],
                )
                .unwrap();
        }
    }

    trait OwnedFence {
        fn owned_fence(self) -> LaneInitializationFence;
    }

    impl OwnedFence for LaneInitializationClaim {
        fn owned_fence(self) -> LaneInitializationFence {
            let LaneInitializationClaim::Owned { fence, .. } = self else {
                panic!("expected owned lane initialization claim")
            };
            fence
        }
    }

    #[test]
    fn first_claim_inserts_reservation_and_generation_one_owner_atomically() {
        let mut fixture = OwnerFixture::new();
        let claim = fixture.claim("lane-a");
        let LaneInitializationClaim::Owned {
            record,
            fence,
            resumed,
        } = claim
        else {
            panic!()
        };
        assert!(!resumed);
        assert_eq!(record.lane_name, "lane-a");
        assert_eq!(fence.owner_generation, 1);
        assert_eq!(fence.owner_token.len(), 64);
        assert_eq!(fixture.owner_count("lane-a"), 1);
    }

    #[test]
    fn live_owner_is_contended_even_when_heartbeat_is_expired() {
        let mut fixture = OwnerFixture::new();
        let first = fixture.claim("lane-a").owned_fence();
        fixture.age_heartbeat("lane-a", i64::MIN / 2);
        let LaneInitializationClaim::Contended { record, owner_pid } = fixture.claim("lane-a")
        else {
            panic!()
        };
        assert_eq!(record.lane_name, "lane-a");
        assert_eq!(owner_pid, std::process::id());
        assert_eq!(fixture.stored_fence("lane-a"), first);
    }

    #[test]
    fn unknown_owner_is_contended_even_when_heartbeat_is_expired() {
        let mut fixture = OwnerFixture::new();
        let first = fixture.claim("lane-a").owned_fence();
        fixture.age_heartbeat("lane-a", i64::MIN / 2);
        fixture.mark_owner_unknown("lane-a");
        assert!(matches!(
            fixture.claim("lane-a"),
            LaneInitializationClaim::Contended { .. }
        ));
        assert_eq!(fixture.stored_fence("lane-a"), first);
    }

    #[test]
    fn dead_owner_takeover_is_cas_fenced_and_increments_generation() {
        let mut fixture = OwnerFixture::new();
        let first = fixture.claim("lane-a").owned_fence();
        fixture.mark_owner_dead("lane-a");
        let second = fixture.claim("lane-a").owned_fence();
        assert_ne!(second.owner_token, first.owner_token);
        assert_eq!(second.owner_generation, first.owner_generation + 1);
        assert!(!fixture.owner_fence_matches("lane-a", &first));
    }

    #[test]
    fn pid_reuse_with_a_different_start_identity_is_takeover_not_contention() {
        let mut fixture = OwnerFixture::new();
        fixture.install_owner(std::process::id(), "different-start-token");
        assert!(matches!(
            fixture.claim("lane-a"),
            LaneInitializationClaim::Owned { resumed: true, .. }
        ));
    }

    #[test]
    fn terminal_initialization_replays_without_creating_an_owner() {
        let mut fixture = OwnerFixture::new();
        fixture.install_terminal("lane-a", LaneInitializationPhase::ObserverReady);
        let LaneInitializationClaim::Terminal(record) = fixture.claim("lane-a") else {
            panic!()
        };
        assert_eq!(record.phase, LaneInitializationPhase::ObserverReady);
        assert_eq!(fixture.owner_count("lane-a"), 0);
    }

    #[test]
    fn heartbeat_release_and_fence_checks_require_the_exact_fence() {
        let mut fixture = OwnerFixture::new();
        let fence = fixture.claim("lane-a").owned_fence();
        fixture.age_heartbeat("lane-a", i64::MIN / 2);
        heartbeat_lane_initialization_owner(
            &fixture.db.conn,
            &fixture.request.initialization_id,
            &fence,
        )
        .unwrap();
        let heartbeat: i64 = fixture
            .db
            .conn
            .query_row(
                "SELECT heartbeat_at FROM lane_initialization_owners
                 WHERE initialization_id=?1",
                [&fixture.request.initialization_id],
                |row| row.get(0),
            )
            .unwrap();
        assert!(heartbeat > i64::MIN / 2);

        let stale = LaneInitializationFence {
            owner_token: fence.owner_token.clone(),
            owner_generation: fence.owner_generation + 1,
        };
        assert!(heartbeat_lane_initialization_owner(
            &fixture.db.conn,
            &fixture.request.initialization_id,
            &stale,
        )
        .is_err());
        assert!(!release_lane_initialization_owner(
            &fixture.db.conn,
            &fixture.request.initialization_id,
            &stale,
        )
        .unwrap());
        assert!(release_lane_initialization_owner(
            &fixture.db.conn,
            &fixture.request.initialization_id,
            &fence,
        )
        .unwrap());
        assert!(!fixture.owner_fence_matches("lane-a", &fence));
    }

    #[test]
    fn dead_repair_owner_takeover_is_cas_fenced_and_increments_generation() {
        let mut fixture = OwnerFixture::new();
        fixture.install_repair_owner(u32::MAX, "dead-repair-owner", i64::MIN / 2);
        install_process_liveness_override(u32::MAX, "dead-repair-owner", false);

        let LaneInitializationRepairClaim::Owned { record, fence } = fixture.claim_repair() else {
            panic!("expected repair ownership takeover")
        };
        assert_eq!(record.phase, LaneInitializationPhase::RepairRequired);
        assert_eq!(fence.owner_generation, 2);
        assert_eq!(fixture.stored_fence("lane-a"), fence);
    }

    #[test]
    fn dead_associated_owner_takeover_transitions_repair_and_increments_generation() {
        let mut fixture = OwnerFixture::new();
        fixture.install_associated_owner(u32::MAX, "dead-associated-owner", i64::MIN / 2);
        install_process_liveness_override(u32::MAX, "dead-associated-owner", false);

        let LaneInitializationRepairClaim::Owned { record, fence } = fixture.claim_repair() else {
            panic!("expected associated ownership takeover")
        };
        assert_eq!(record.phase, LaneInitializationPhase::RepairRequired);
        assert_eq!(fence.owner_generation, 2);
        assert_eq!(fixture.stored_fence("lane-a"), fence);
    }

    #[test]
    fn live_and_unknown_repair_owners_are_never_stolen_by_heartbeat_age() {
        for (phase, start_identity, unknown) in [
            ("repair_required", "live-repair-owner", false),
            ("repair_required", "unknown-repair-owner", true),
            ("associated", "live-associated-owner", false),
            ("associated", "unknown-associated-owner", true),
        ] {
            let mut fixture = OwnerFixture::new();
            fixture.install_active_owner(phase, std::process::id(), start_identity, i64::MIN / 2);
            if unknown {
                install_process_liveness_unknown_override(std::process::id(), start_identity);
            } else {
                install_process_liveness_override(std::process::id(), start_identity, true);
            }
            let before = fixture.stored_fence("lane-a");
            let LaneInitializationRepairClaim::Contended { owner_pid, .. } = fixture.claim_repair()
            else {
                panic!("expected live repair-owner contention")
            };
            assert_eq!(owner_pid, std::process::id());
            assert_eq!(fixture.stored_fence("lane-a"), before);
        }
    }
}
