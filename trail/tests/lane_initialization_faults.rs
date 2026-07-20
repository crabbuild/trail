use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Barrier, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use trail::{Actor, InitImportMode, LaneInitializationPhase, LaneWorkdirMode, Trail};

const CHILD_WORKSPACE: &str = "TRAIL_TEST_INITIALIZATION_CHILD_WORKSPACE";
const CHILD_LEDGER_AUTHORITY: &str = "TRAIL_TEST_INITIALIZATION_CHILD_LEDGER_AUTHORITY";
const CRASH_AFTER: &str = "TRAIL_TEST_LANE_INITIALIZATION_CRASH_AFTER";
const HANDSHAKE: &str = "TRAIL_TEST_LANE_INITIALIZATION_HANDSHAKE";
const CHILD_EXPECT_SUCCESS: &str = "TRAIL_TEST_LANE_INITIALIZATION_EXPECT_SUCCESS";
const CHILD_REPAIR_LANE: &str = "TRAIL_TEST_LANE_INITIALIZATION_REPAIR_LANE";
const CHILD_READY: &str = "TRAIL_TEST_LANE_INITIALIZATION_CHILD_READY";
const CHILD_START: &str = "TRAIL_TEST_LANE_INITIALIZATION_CHILD_START";

fn initialize_workspace(path: &Path) {
    Trail::init(path, "main", InitImportMode::Empty, false).unwrap();
    fs::write(path.join("file.txt"), "durable initialization\n").unwrap();
    let mut db = Trail::open(path).unwrap();
    db.record(
        Some("main"),
        Some("initialize crash fixture".into()),
        Actor::human(),
        false,
    )
    .unwrap();
}

fn copy_directory(source: &Path, destination: &Path) {
    fs::create_dir(destination).unwrap();
    for entry in fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_directory(&source_path, &destination_path);
        } else {
            fs::copy(source_path, destination_path).unwrap();
        }
    }
}

fn count(sqlite: &Path, sql: &str, parameter: &str) -> i64 {
    Connection::open(sqlite)
        .unwrap()
        .query_row(sql, [parameter], |row| row.get(0))
        .unwrap()
}

fn rows_for_lane(sqlite: &Path, lane: &str) -> i64 {
    count(
        sqlite,
        "SELECT COUNT(*) FROM lane_initializations WHERE lane_name=?1",
        lane,
    )
}

fn owners_for_lane(sqlite: &Path, lane: &str) -> i64 {
    count(
        sqlite,
        "SELECT COUNT(*) FROM lane_initialization_owners owner
         JOIN lane_initializations initialization
           ON initialization.initialization_id=owner.initialization_id
         WHERE initialization.lane_name=?1",
        lane,
    )
}

fn refs_for_lane(sqlite: &Path, lane: &str) -> i64 {
    count(
        sqlite,
        "SELECT COUNT(*) FROM refs WHERE name=?1",
        &format!("refs/lanes/{lane}"),
    )
}

fn spawn_events_for_lane(sqlite: &Path, lane: &str) -> i64 {
    count(
        sqlite,
        "SELECT COUNT(*) FROM lane_events event JOIN lanes lane ON lane.lane_id=event.lane_id WHERE lane.name=?1 AND event.event_type='lane_spawned'",
        lane,
    )
}

fn lane_initialization_filesystem_artifacts(root: &Path) -> Vec<String> {
    fn visit(root: &Path, current: &Path, paths: &mut Vec<String>) {
        for entry in fs::read_dir(current).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name == "lane-initialization-locks"
                || name.starts_with("lane-initialization-")
                || name.starts_with(".lane-initialization-candidate-")
            {
                paths.push(
                    path.strip_prefix(root)
                        .unwrap()
                        .to_string_lossy()
                        .into_owned(),
                );
            }
            if entry.file_type().unwrap().is_dir() {
                visit(root, &path, paths);
            }
        }
    }

    let mut paths = Vec::new();
    visit(root, root, &mut paths);
    paths.sort();
    paths.dedup();
    paths
}

fn active_owner_rows(sqlite: &Path) -> i64 {
    Connection::open(sqlite)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM lane_initialization_owners",
            [],
            |row| row.get(0),
        )
        .unwrap()
}

fn reject_lane_initialization_phases(sqlite: &Path, phases: &[&str]) {
    let phases = phases
        .iter()
        .map(|phase| format!("'{phase}'"))
        .collect::<Vec<_>>()
        .join(",");
    Connection::open(sqlite)
        .unwrap()
        .execute_batch(&format!(
            "CREATE TRIGGER reject_lane_initialization_phase
             BEFORE UPDATE OF phase ON lane_initializations
             WHEN NEW.phase IN ({phases})
             BEGIN
               SELECT RAISE(FAIL, 'injected lane initialization phase persistence failure');
             END;"
        ))
        .unwrap();
}

fn reject_lane_initialization_owner_deletes(sqlite: &Path) {
    Connection::open(sqlite)
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER reject_lane_initialization_owner_delete
             BEFORE DELETE ON lane_initialization_owners
             BEGIN
               SELECT RAISE(FAIL, 'injected lane initialization owner release failure');
             END;",
        )
        .unwrap();
}

fn ownerless_associated_fixture(lane: &str) -> (tempfile::TempDir, Trail) {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let mut db = Trail::open(temp.path()).unwrap();
    reject_lane_initialization_phases(&sqlite, &["repair_required"]);
    trail::test_support::set_lane_association_failure_for_current_thread(Some("spawn_event"));
    let error = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            lane,
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap_err();
    trail::test_support::set_lane_association_failure_for_current_thread(None);
    assert!(matches!(
        error,
        trail::Error::CommittedRepairRequired {
            committed: true,
            phase: LaneInitializationPhase::Associated,
            ..
        }
    ));
    Connection::open(&sqlite)
        .unwrap()
        .execute_batch("DROP TRIGGER reject_lane_initialization_phase;")
        .unwrap();
    assert_eq!(owners_for_lane(&sqlite, lane), 0);
    assert_eq!(spawn_events_for_lane(&sqlite, lane), 0);
    (temp, db)
}

fn ownerless_materialized_repair_fixture(lane: &str) -> (tempfile::TempDir, Trail, PathBuf) {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let mut db = Trail::open(temp.path()).unwrap();
    trail::test_support::set_lane_association_failure_for_current_thread(Some(
        "spawn_journal_completion",
    ));
    let error = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            lane,
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap_err();
    trail::test_support::set_lane_association_failure_for_current_thread(None);
    assert!(matches!(
        error,
        trail::Error::CommittedRepairRequired {
            committed: true,
            phase: LaneInitializationPhase::RepairRequired,
            ..
        }
    ));
    let initialization = db.lane_initialization(lane).unwrap().unwrap();
    let journal = temp
        .path()
        .join(".trail/materialization-operations")
        .join(format!("{}.json", initialization.operation_id));
    assert!(journal.is_file());
    assert_eq!(owners_for_lane(&sqlite, lane), 0);
    assert_eq!(spawn_events_for_lane(&sqlite, lane), 0);
    (temp, db, journal)
}

#[test]
fn lane_initialization_crash_child() {
    let Ok(workspace) = std::env::var(CHILD_WORKSPACE) else {
        return;
    };
    if std::env::var_os(CHILD_LEDGER_AUTHORITY).is_some() {
        trail::test_support::set_changed_path_authority_override(true);
    }
    let mut db = Trail::open(workspace).unwrap();
    if let (Some(ready), Some(start)) =
        (std::env::var_os(CHILD_READY), std::env::var_os(CHILD_START))
    {
        fs::write(ready, b"ready\n").unwrap();
        let start = PathBuf::from(start);
        let deadline = Instant::now() + Duration::from_secs(10);
        while !start.exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            start.exists(),
            "process coordination start barrier timed out"
        );
    }
    let result = if let Ok(lane) = std::env::var(CHILD_REPAIR_LANE) {
        db.repair_lane_initialization(&lane)
    } else {
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "crash-lane",
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            Some("codex".into()),
            Some("gpt-5".into()),
            None,
            &[],
            false,
        )
    };
    if std::env::var_os(CHILD_EXPECT_SUCCESS).is_some() {
        result.unwrap();
    }
}

fn initialization_child(workspace: &Path) -> Command {
    let mut child = Command::new(std::env::current_exe().unwrap());
    child
        .arg("--exact")
        .arg("lane_initialization_crash_child")
        .arg("--nocapture")
        .env(CHILD_WORKSPACE, workspace)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    child
}

#[test]
fn quiescent_workspace_copy_replays_without_filesystem_authority() {
    let source = tempfile::tempdir().unwrap();
    initialize_workspace(source.path());
    {
        let mut db = Trail::open(source.path()).unwrap();
        db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "relocated-lane",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    }

    let destination = tempfile::tempdir().unwrap();
    let copied_workspace = destination.path().join("copied-workspace");
    copy_directory(source.path(), &copied_workspace);
    let mut copied = Trail::open(&copied_workspace).unwrap();
    let report = copied
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "relocated-lane",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    assert_eq!(report.phase, LaneInitializationPhase::ObserverReady);
    assert!(report.resumed);
    let trail_root = copied_workspace.join(".trail");
    assert!(lane_initialization_filesystem_artifacts(&trail_root).is_empty());
    assert_eq!(active_owner_rows(&trail_root.join("index/trail.sqlite")), 0);
}

#[test]
fn lane_initialization_creates_no_filesystem_coordination_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let trail_root = temp.path().join(".trail");
    let before = lane_initialization_filesystem_artifacts(&trail_root);

    let mut db = Trail::open(temp.path()).unwrap();
    db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "no-filesystem-authority",
        Some("main"),
        LaneWorkdirMode::Virtual,
        None,
        None,
        None,
        &[],
        false,
    )
    .unwrap();

    assert_eq!(
        lane_initialization_filesystem_artifacts(&trail_root),
        before
    );
    assert_eq!(active_owner_rows(&trail_root.join("index/trail.sqlite")), 0);
}

struct MaterializationRelease {
    state: Arc<(Mutex<bool>, Condvar)>,
}

impl Drop for MaterializationRelease {
    fn drop(&mut self) {
        let (released, changed) = &*self.state;
        *released.lock().unwrap_or_else(|poison| poison.into_inner()) = true;
        changed.notify_all();
    }
}

#[test]
fn authority_off_distinct_materializations_overlap_without_workspace_lock() {
    const WORKERS: usize = 2;
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let workspace = Arc::new(temp.path().to_path_buf());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let start = Arc::new(Barrier::new(WORKERS + 1));
    let (entered_tx, entered_rx) = mpsc::channel();
    let release_state = Arc::new((Mutex::new(false), Condvar::new()));
    let release = MaterializationRelease {
        state: Arc::clone(&release_state),
    };
    let handles = (0..WORKERS)
        .map(|index| {
            let workspace = Arc::clone(&workspace);
            let start = Arc::clone(&start);
            let entered = entered_tx.clone();
            let release_state = Arc::clone(&release_state);
            thread::spawn(move || {
                trail::test_support::set_changed_path_authority_override(false);
                let mut db = Trail::open(&*workspace).unwrap();
                trail::test_support::set_lane_initialization_materialization_barrier_for_current_thread(
                    Some((entered, release_state)),
                );
                start.wait();
                let lane = format!("authority-off-overlap-{index}");
                let result = db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                    &lane,
                    Some("main"),
                    LaneWorkdirMode::PortableCopy,
                    None,
                    None,
                    None,
                    &[],
                    false,
                );
                trail::test_support::set_lane_initialization_materialization_barrier_for_current_thread(None);
                (lane, result)
            })
        })
        .collect::<Vec<_>>();
    drop(entered_tx);
    start.wait();

    for _ in 0..WORKERS {
        entered_rx
            .recv_timeout(Duration::from_secs(10))
            .expect("distinct materializations did not overlap at the deterministic barrier");
    }
    assert!(
        !temp.path().join(".trail/lock").exists(),
        "lane coordination created or held the workspace lock during materialization"
    );
    drop(release);

    for handle in handles {
        let (lane, result) = handle.join().unwrap();
        let report = result.unwrap_or_else(|error| panic!("{lane} failed: {error:?}"));
        assert_eq!(report.phase, LaneInitializationPhase::ObserverReady);
        assert!(report
            .workdir
            .as_ref()
            .is_some_and(|path| Path::new(path).is_dir()));
        assert_eq!(owners_for_lane(&sqlite, &lane), 0);
    }
    assert_eq!(active_owner_rows(&sqlite), 0);
    assert!(!temp.path().join(".trail/lock").exists());
}

#[test]
fn authority_on_claim_reaches_materialization_before_workspace_lock() {
    struct AuthorityOverride;
    impl Drop for AuthorityOverride {
        fn drop(&mut self) {
            trail::test_support::set_changed_path_authority_override(false);
        }
    }

    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let workspace = temp.path().to_path_buf();
    let (entered_tx, entered_rx) = mpsc::channel();
    let release_state = Arc::new((Mutex::new(false), Condvar::new()));
    let release = MaterializationRelease {
        state: Arc::clone(&release_state),
    };
    let worker = thread::spawn(move || {
        trail::test_support::set_changed_path_authority_override(true);
        let _authority = AuthorityOverride;
        let mut db = Trail::open(&workspace).unwrap();
        trail::test_support::set_lane_initialization_materialization_barrier_for_current_thread(
            Some((entered_tx, release_state)),
        );
        let result = db.spawn_lane_with_workdir_mode_paths_and_neighbors(
            "authority-on-claim",
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            None,
            None,
            None,
            &[],
            false,
        );
        trail::test_support::set_lane_initialization_materialization_barrier_for_current_thread(
            None,
        );
        result
    });

    entered_rx
        .recv_timeout(Duration::from_secs(10))
        .expect("authority-on claim did not reach the pre-materialization barrier");
    assert!(
        !temp.path().join(".trail/lock").exists(),
        "owner claim acquired the workspace lock before materialization"
    );
    drop(release);
    assert_eq!(
        worker.join().unwrap().unwrap().phase,
        LaneInitializationPhase::ObserverReady
    );
    assert!(!temp.path().join(".trail/lock").exists());
}

fn crash_and_resume(boundary: &str) {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let handshake = temp.path().join(format!("{boundary}.handshake"));
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("lane_initialization_crash_child")
        .arg("--nocapture")
        .env(CHILD_WORKSPACE, temp.path())
        .env(CRASH_AFTER, boundary)
        .env(HANDSHAKE, &handshake)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !handshake.exists() && Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        handshake.exists(),
        "child did not publish {boundary} handshake"
    );
    child.kill().unwrap();
    child.wait().unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let result = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "crash-lane",
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            Some("codex".into()),
            Some("gpt-5".into()),
            None,
            &[],
            false,
        )
        .unwrap();
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    assert_eq!(rows_for_lane(&sqlite, "crash-lane"), 1);
    assert_eq!(refs_for_lane(&sqlite, "crash-lane"), 1);
    assert_eq!(spawn_events_for_lane(&sqlite, "crash-lane"), 1);
    assert_eq!(result.phase, LaneInitializationPhase::ObserverReady);
    assert!(result.resumed);
}

#[test]
fn identical_spawn_resumes_at_every_durable_crash_cut() {
    for boundary in [
        "after_reservation",
        "after_materialization",
        "after_association",
        "after_reconciliation",
        "after_marker",
        "after_spawn_event",
    ] {
        crash_and_resume(boundary);
    }
}

#[test]
fn dead_process_owner_is_taken_over_and_replays_after_every_crash_cut() {
    for boundary in [
        "repair_after_ref_mirror",
        "repair_after_journal",
        "repair_before_observer_ready",
    ] {
        let lane = "repair-crash";
        let (temp, db, _journal) = ownerless_materialized_repair_fixture(lane);
        drop(db);
        let sqlite = temp.path().join(".trail/index/trail.sqlite");
        let handshake = temp.path().join(format!("{boundary}.handshake"));
        let mut child = initialization_child(temp.path())
            .env(CHILD_REPAIR_LANE, lane)
            .env(CRASH_AFTER, boundary)
            .env(HANDSHAKE, &handshake)
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !handshake.exists() && Instant::now() < deadline {
            if child.try_wait().unwrap().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(handshake.exists(), "repair winner did not reach {boundary}");
        let generation: i64 = Connection::open(&sqlite)
            .unwrap()
            .query_row(
                "SELECT owner.owner_generation
                 FROM lane_initialization_owners owner
                 JOIN lane_initializations initialization
                   ON initialization.initialization_id=owner.initialization_id
                 WHERE initialization.lane_name=?1",
                [lane],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(generation, 1);
        child.kill().unwrap();
        child.wait().unwrap();

        let mut survivor = Trail::open(temp.path()).unwrap();
        let report = survivor.repair_lane_initialization(lane).unwrap();
        assert_eq!(report.phase, LaneInitializationPhase::ObserverReady);
        assert!(report.resumed);
        assert_eq!(owners_for_lane(&sqlite, lane), 0);
        assert_eq!(spawn_events_for_lane(&sqlite, lane), 1);
        assert_eq!(refs_for_lane(&sqlite, lane), 1);
    }
}

#[test]
fn dead_associated_owner_is_atomically_taken_over_for_repair() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let associated = temp.path().join("associated-owner.handshake");
    let mut spawn_winner = initialization_child(temp.path())
        .env(CRASH_AFTER, "after_association")
        .env(HANDSHAKE, &associated)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !associated.exists() && Instant::now() < deadline {
        if spawn_winner.try_wait().unwrap().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        associated.exists(),
        "spawn winner did not associate the lane"
    );
    let original: (String, i64) = Connection::open(&sqlite)
        .unwrap()
        .query_row(
            "SELECT owner.owner_token,owner.owner_generation
             FROM lane_initialization_owners owner
             JOIN lane_initializations initialization
               ON initialization.initialization_id=owner.initialization_id
             WHERE initialization.lane_name='crash-lane'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(original.1, 1);
    spawn_winner.kill().unwrap();
    spawn_winner.wait().unwrap();

    let repair_cut = temp.path().join("associated-repair-takeover.handshake");
    let mut repair_winner = initialization_child(temp.path())
        .env(CHILD_REPAIR_LANE, "crash-lane")
        .env(CRASH_AFTER, "repair_after_ref_mirror")
        .env(HANDSHAKE, &repair_cut)
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while !repair_cut.exists() && Instant::now() < deadline {
        if repair_winner.try_wait().unwrap().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(repair_cut.exists(), "repair winner did not take over");
    let takeover: (String, i64, String) = Connection::open(&sqlite)
        .unwrap()
        .query_row(
            "SELECT owner.owner_token,owner.owner_generation,initialization.phase
             FROM lane_initialization_owners owner
             JOIN lane_initializations initialization
               ON initialization.initialization_id=owner.initialization_id
             WHERE initialization.lane_name='crash-lane'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_ne!(takeover.0, original.0);
    assert_eq!(takeover.1, original.1 + 1);
    assert_eq!(takeover.2, "repair_required");
    repair_winner.kill().unwrap();
    repair_winner.wait().unwrap();

    let mut survivor = Trail::open(temp.path()).unwrap();
    let report = survivor.repair_lane_initialization("crash-lane").unwrap();
    assert_eq!(report.phase, LaneInitializationPhase::ObserverReady);
    assert_eq!(owners_for_lane(&sqlite, "crash-lane"), 0);
    assert_eq!(refs_for_lane(&sqlite, "crash-lane"), 1);
    assert_eq!(spawn_events_for_lane(&sqlite, "crash-lane"), 1);
}

#[test]
fn live_owner_timeout_is_stable_and_never_revokes() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let lane = "live-timeout";
    let mut db = Trail::open(temp.path()).unwrap();
    trail::test_support::set_lane_association_failure_for_current_thread(Some("spawn_after_ref"));
    let failed = db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        lane,
        Some("main"),
        LaneWorkdirMode::PortableCopy,
        Some("codex".into()),
        Some("gpt-5".into()),
        None,
        &[],
        false,
    );
    trail::test_support::set_lane_association_failure_for_current_thread(None);
    assert!(failed.is_err());
    assert_eq!(owners_for_lane(&sqlite, lane), 0);

    let initialization_id: String = Connection::open(&sqlite)
        .unwrap()
        .query_row(
            "SELECT initialization_id FROM lane_initializations WHERE lane_name=?1",
            [lane],
            |row| row.get(0),
        )
        .unwrap();
    let owner_token = "44".repeat(32);
    let owner_generation = 7_i64;
    Connection::open(&sqlite)
        .unwrap()
        .execute(
            "INSERT INTO lane_initialization_owners(
                 initialization_id,owner_token,owner_generation,owner_pid,
                 owner_process_start_identity,acquired_at,heartbeat_at)
             VALUES(?1,?2,?3,?4,?5,1,1)",
            rusqlite::params![
                initialization_id,
                owner_token,
                owner_generation,
                std::process::id(),
                trail::test_support::current_process_start_token(),
            ],
        )
        .unwrap();

    trail::test_support::set_lane_initialization_wait_timeout_for_current_thread(Some(
        Duration::ZERO,
    ));
    let error = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            lane,
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            Some("codex".into()),
            Some("gpt-5".into()),
            None,
            &[],
            false,
        )
        .unwrap_err();
    trail::test_support::set_lane_initialization_wait_timeout_for_current_thread(None);
    assert_eq!(error.code(), "LANE_INITIALIZATION_IN_PROGRESS");
    assert_eq!(error.exit_code(), 2);
    let trail::Error::LaneInitializationInProgress { retry_command, .. } = &error else {
        panic!("expected stable in-progress error")
    };
    assert!(retry_command.contains("repeat the exact original"));
    let stored: (String, i64, u32, i64) = Connection::open(&sqlite)
        .unwrap()
        .query_row(
            "SELECT owner_token,owner_generation,owner_pid,heartbeat_at
             FROM lane_initialization_owners WHERE initialization_id=?1",
            [&initialization_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        stored,
        (owner_token, owner_generation, std::process::id(), 1)
    );
}

#[test]
fn lost_owner_fence_restarts_claim_and_replay_internally() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let mut db = Trail::open(temp.path()).unwrap();
    trail::test_support::steal_lane_initialization_owner_on_next_heartbeat_for_current_thread();
    let report = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "lost-fence-retry",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    assert_eq!(report.phase, LaneInitializationPhase::ObserverReady);
    assert!(report.resumed);
    assert_eq!(owners_for_lane(&sqlite, "lost-fence-retry"), 0);
    assert_eq!(refs_for_lane(&sqlite, "lost-fence-retry"), 1);
    assert_eq!(spawn_events_for_lane(&sqlite, "lost-fence-retry"), 1);
}

#[test]
fn stale_owner_cannot_transition_after_generation_takeover() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let handshake = temp.path().join("stale-owner.handshake");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("lane_initialization_crash_child")
        .arg("--nocapture")
        .env(CHILD_WORKSPACE, temp.path())
        .env(CRASH_AFTER, "after_reservation")
        .env(HANDSHAKE, &handshake)
        .env(CHILD_LEDGER_AUTHORITY, "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !handshake.exists() && Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(handshake.exists(), "child did not publish owner handshake");

    let (initialization_id, stale_token, stale_generation): (String, String, i64) =
        Connection::open(&sqlite)
            .unwrap()
            .query_row(
                "SELECT initialization.initialization_id,owner.owner_token,owner.owner_generation
                 FROM lane_initializations initialization
                 JOIN lane_initialization_owners owner
                   ON owner.initialization_id=initialization.initialization_id
                 WHERE initialization.lane_name='crash-lane'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
    child.kill().unwrap();
    child.wait().unwrap();

    let replacement_token = "22".repeat(32);
    let changed = Connection::open(&sqlite)
        .unwrap()
        .execute(
            "UPDATE lane_initialization_owners
             SET owner_token=?1,owner_generation=owner_generation+1,
                 owner_pid=?2,owner_process_start_identity='replacement-owner'
             WHERE initialization_id=?3 AND owner_token=?4 AND owner_generation=?5",
            rusqlite::params![
                replacement_token,
                std::process::id(),
                initialization_id,
                stale_token,
                stale_generation,
            ],
        )
        .unwrap();
    assert_eq!(changed, 1);

    let mut db = Trail::open(temp.path()).unwrap();
    let error = db
        .debug_transition_lane_initialization_with_fence(
            &initialization_id,
            &stale_token,
            stale_generation,
            LaneInitializationPhase::Reserved,
            LaneInitializationPhase::Materialized,
        )
        .unwrap_err();
    assert!(error.to_string().contains("owner fence no longer matches"));
    assert_eq!(
        db.lane_initialization("crash-lane").unwrap().unwrap().phase,
        LaneInitializationPhase::Reserved
    );
    let stored_generation: i64 = Connection::open(&sqlite)
        .unwrap()
        .query_row(
            "SELECT owner_generation FROM lane_initialization_owners
             WHERE initialization_id=?1",
            [&initialization_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored_generation, stale_generation + 1);
}

#[test]
fn pre_association_rollback_retains_materialized_state_and_releases_exact_owner() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let mut db = Trail::open(temp.path()).unwrap();
    trail::test_support::set_lane_association_failure_for_current_thread(Some("spawn_after_ref"));
    let result = db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "retained-materialized",
        Some("main"),
        LaneWorkdirMode::PortableCopy,
        None,
        None,
        None,
        &[],
        false,
    );
    trail::test_support::set_lane_association_failure_for_current_thread(None);
    assert!(result.is_err());

    let initialization = db
        .lane_initialization("retained-materialized")
        .unwrap()
        .unwrap();
    assert_eq!(initialization.phase, LaneInitializationPhase::Materialized);
    assert_eq!(owners_for_lane(&sqlite, "retained-materialized"), 0);
    assert_eq!(refs_for_lane(&sqlite, "retained-materialized"), 0);
    assert_eq!(spawn_events_for_lane(&sqlite, "retained-materialized"), 0);
    assert!(
        initialization
            .workdir
            .as_ref()
            .is_some_and(|workdir| Path::new(workdir).is_dir()),
        "materialized rollback must retain its reusable workdir"
    );
}

#[test]
fn stale_owner_cannot_publish_lane_spawn_event_after_takeover() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let handshake = temp.path().join("stale-event-owner.handshake");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("lane_initialization_crash_child")
        .arg("--nocapture")
        .env(CHILD_WORKSPACE, temp.path())
        .env(CRASH_AFTER, "after_association")
        .env(HANDSHAKE, &handshake)
        .env(CHILD_LEDGER_AUTHORITY, "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !handshake.exists() && Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        handshake.exists(),
        "child did not publish association handshake"
    );
    let (initialization_id, stale_token, stale_generation): (String, String, i64) =
        Connection::open(&sqlite)
            .unwrap()
            .query_row(
                "SELECT initialization.initialization_id,owner.owner_token,owner.owner_generation
                 FROM lane_initializations initialization
                 JOIN lane_initialization_owners owner
                   ON owner.initialization_id=initialization.initialization_id
                 WHERE initialization.lane_name='crash-lane'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
    child.kill().unwrap();
    child.wait().unwrap();
    Connection::open(&sqlite)
        .unwrap()
        .execute(
            "UPDATE lane_initialization_owners
             SET owner_token=?1,owner_generation=owner_generation+1,
                 owner_pid=?2,owner_process_start_identity='replacement-event-owner'
             WHERE initialization_id=?3 AND owner_token=?4 AND owner_generation=?5",
            rusqlite::params![
                "33".repeat(32),
                std::process::id(),
                initialization_id,
                stale_token,
                stale_generation,
            ],
        )
        .unwrap();

    let mut db = Trail::open(temp.path()).unwrap();
    let error = db
        .debug_publish_lane_spawn_event_with_fence("crash-lane", &stale_token, stale_generation)
        .unwrap_err();
    assert!(error.to_string().contains("owner fence no longer matches"));
    assert_eq!(spawn_events_for_lane(&sqlite, "crash-lane"), 0);
}

#[test]
fn live_and_unknown_associated_owner_timeout_preserves_fence_and_side_effects() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let handshake = temp.path().join("live-repair-owner.handshake");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("lane_initialization_crash_child")
        .arg("--nocapture")
        .env(CHILD_WORKSPACE, temp.path())
        .env(CRASH_AFTER, "after_association")
        .env(HANDSHAKE, &handshake)
        .env(CHILD_LEDGER_AUTHORITY, "1")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !handshake.exists() && Instant::now() < deadline {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        handshake.exists(),
        "child did not publish live-owner handshake"
    );

    let before: (String, i64, u32, String, i64) = Connection::open(&sqlite)
        .unwrap()
        .query_row(
            "SELECT owner.owner_token,owner.owner_generation,owner.owner_pid,
                    owner.owner_process_start_identity,owner.heartbeat_at
             FROM lane_initialization_owners owner
             JOIN lane_initializations initialization
               ON initialization.initialization_id=owner.initialization_id
             WHERE initialization.lane_name='crash-lane'",
            [],
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
    let mut db = Trail::open(temp.path()).unwrap();
    trail::test_support::set_lane_initialization_wait_timeout_for_current_thread(Some(
        Duration::ZERO,
    ));
    trail::test_support::set_lane_association_failure_for_current_thread(Some("spawn_ref_repair"));
    for unknown in [false, true] {
        if unknown {
            trail::test_support::set_lane_initialization_owner_liveness_unknown_for_current_thread(
                before.2, &before.3,
            );
        }
        let error = db.repair_lane_initialization("crash-lane").unwrap_err();
        assert_eq!(error.code(), "LANE_INITIALIZATION_IN_PROGRESS");
        let trail::Error::LaneInitializationInProgress { retry_command, .. } = &error else {
            panic!("expected stable repair contention")
        };
        assert_eq!(retry_command, "trail lane repair-initialization crash-lane");
        assert!(!error
            .to_string()
            .contains("injected lane association failure"));
    }
    trail::test_support::set_lane_association_failure_for_current_thread(None);
    trail::test_support::set_lane_initialization_wait_timeout_for_current_thread(None);
    trail::test_support::clear_lane_initialization_owner_liveness_overrides_for_current_thread();
    assert_eq!(
        db.lane_initialization("crash-lane").unwrap().unwrap().phase,
        LaneInitializationPhase::Associated
    );
    assert_eq!(spawn_events_for_lane(&sqlite, "crash-lane"), 0);
    let after: (String, i64, u32, String, i64) = Connection::open(&sqlite)
        .unwrap()
        .query_row(
            "SELECT owner.owner_token,owner.owner_generation,owner.owner_pid,
                    owner.owner_process_start_identity,owner.heartbeat_at
             FROM lane_initialization_owners owner
             JOIN lane_initializations initialization
               ON initialization.initialization_id=owner.initialization_id
             WHERE initialization.lane_name='crash-lane'",
            [],
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
    assert_eq!(after, before);
    child.kill().unwrap();
    child.wait().unwrap();
}

#[test]
fn concurrent_ownerless_repair_publishes_one_event_and_replays_idempotently() {
    const REPAIRERS: usize = 8;
    let (temp, db, journal) = ownerless_materialized_repair_fixture("ownerless-repair");
    drop(db);
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let workspace = Arc::new(temp.path().to_path_buf());
    let start = Arc::new(Barrier::new(REPAIRERS + 1));
    let handles = (0..REPAIRERS)
        .map(|_| {
            let workspace = Arc::clone(&workspace);
            let start = Arc::clone(&start);
            thread::spawn(move || {
                let mut db = Trail::open(workspace.as_ref()).unwrap();
                start.wait();
                db.repair_lane_initialization("ownerless-repair")
            })
        })
        .collect::<Vec<_>>();
    start.wait();
    for handle in handles {
        let report = handle.join().unwrap().unwrap();
        assert_eq!(report.phase, LaneInitializationPhase::ObserverReady);
    }

    let mut db = Trail::open(temp.path()).unwrap();
    let replay = db.repair_lane_initialization("ownerless-repair").unwrap();
    assert_eq!(replay.phase, LaneInitializationPhase::ObserverReady);
    assert_eq!(spawn_events_for_lane(&sqlite, "ownerless-repair"), 1);
    assert_eq!(owners_for_lane(&sqlite, "ownerless-repair"), 0);
    assert_eq!(
        db.lane_initialization("ownerless-repair")
            .unwrap()
            .unwrap()
            .phase,
        LaneInitializationPhase::ObserverReady
    );
    assert!(!journal.exists());
}

#[test]
fn associated_ownerless_repair_completes_observer_ready() {
    let (temp, mut db) = ownerless_associated_fixture("associated-ownerless-repair");
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let report = db
        .repair_lane_initialization("associated-ownerless-repair")
        .unwrap();
    assert_eq!(report.phase, LaneInitializationPhase::ObserverReady);
    assert_eq!(
        spawn_events_for_lane(&sqlite, "associated-ownerless-repair"),
        1
    );
    assert_eq!(owners_for_lane(&sqlite, "associated-ownerless-repair"), 0);
}

#[test]
fn failed_claimed_repair_persists_error_and_releases_exact_owner() {
    let (temp, mut db, _journal) = ownerless_materialized_repair_fixture("failed-repair-owner");
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let workdir = db
        .lane_details("failed-repair-owner")
        .unwrap()
        .branch
        .workdir
        .unwrap();
    fs::remove_dir_all(workdir).unwrap();

    let error = db
        .repair_lane_initialization("failed-repair-owner")
        .unwrap_err();
    assert!(matches!(
        error,
        trail::Error::CommittedRepairRequired {
            committed: true,
            phase: LaneInitializationPhase::RepairRequired,
            ..
        }
    ));
    assert_eq!(owners_for_lane(&sqlite, "failed-repair-owner"), 0);
    let (phase, code): (String, Option<String>) = Connection::open(&sqlite)
        .unwrap()
        .query_row(
            "SELECT phase,last_error_code FROM lane_initializations WHERE lane_name=?1",
            ["failed-repair-owner"],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(phase, "repair_required");
    assert!(code.is_some());
}

#[cfg(target_os = "macos")]
#[test]
fn transparent_cow_view_failure_persists_repair_and_replays_without_owner() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let lane = "transparent-view-repair";
    let mut db = Trail::open(temp.path()).unwrap();
    trail::test_support::set_lane_association_failure_for_current_thread(Some(
        "spawn_workspace_view",
    ));
    let error = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            lane,
            Some("main"),
            LaneWorkdirMode::NfsCow,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap_err();
    trail::test_support::set_lane_association_failure_for_current_thread(None);
    assert!(matches!(
        error,
        trail::Error::CommittedRepairRequired {
            committed: true,
            phase: LaneInitializationPhase::RepairRequired,
            ..
        }
    ));
    assert_eq!(owners_for_lane(&sqlite, lane), 0);
    assert_eq!(spawn_events_for_lane(&sqlite, lane), 0);
    assert_eq!(
        db.lane_initialization(lane).unwrap().unwrap().phase,
        LaneInitializationPhase::RepairRequired
    );
    assert!(db.lane_workspace_view(lane).unwrap().is_none());

    let repaired = db.repair_lane_initialization(lane).unwrap();
    assert_eq!(repaired.phase, LaneInitializationPhase::ObserverReady);
    assert!(db.lane_workspace_view(lane).unwrap().is_some());
    assert_eq!(owners_for_lane(&sqlite, lane), 0);
    assert_eq!(spawn_events_for_lane(&sqlite, lane), 1);
}

#[test]
fn deferred_materialized_spawn_releases_owner_before_resume() {
    struct AuthorityOverride;
    impl Drop for AuthorityOverride {
        fn drop(&mut self) {
            trail::test_support::set_changed_path_authority_override(false);
        }
    }

    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    trail::test_support::set_changed_path_authority_override(true);
    let _authority = AuthorityOverride;
    let mut db = Trail::open(temp.path()).unwrap();
    let deferred = db
        .spawn_lane_with_deferred_initial_ledger(
            "deferred-owner-release",
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    assert_eq!(deferred.phase, LaneInitializationPhase::Associated);
    assert!(!deferred.completed_deferred_initialization);
    assert_eq!(owners_for_lane(&sqlite, "deferred-owner-release"), 0);
    drop(db);

    let mut reopened = Trail::open(temp.path()).unwrap();
    let completed = reopened
        .resume_deferred_initial_lane_ledger("deferred-owner-release")
        .unwrap();
    assert_eq!(completed.phase, LaneInitializationPhase::ObserverReady);
    assert!(completed.completed_deferred_initialization);
    assert_eq!(owners_for_lane(&sqlite, "deferred-owner-release"), 0);
    assert_eq!(spawn_events_for_lane(&sqlite, "deferred-owner-release"), 1);
}

#[test]
fn io_failures_never_advance_past_or_delete_the_durable_artifact() {
    for (boundary, disk_full) in [
        ("workdir_write", true),
        ("file_sync", true),
        ("directory_sync", false),
        ("association_sqlite_commit", true),
    ] {
        let temp = tempfile::tempdir().unwrap();
        initialize_workspace(temp.path());
        trail::test_support::set_lane_initialization_io_failure_for_current_thread(
            Some(boundary),
            disk_full,
        );
        let mut db = Trail::open(temp.path()).unwrap();
        let error = db
            .spawn_lane_with_workdir_mode_paths_and_neighbors(
                "io-lane",
                Some("main"),
                LaneWorkdirMode::PortableCopy,
                None,
                None,
                None,
                &[],
                false,
            )
            .unwrap_err();
        trail::test_support::set_lane_initialization_io_failure_for_current_thread(None, false);
        assert_eq!(error.code(), "IO_ERROR", "boundary {boundary}: {error}");
        let sqlite = temp.path().join(".trail/index/trail.sqlite");
        assert_eq!(refs_for_lane(&sqlite, "io-lane"), 0, "{boundary}");
        assert_eq!(spawn_events_for_lane(&sqlite, "io-lane"), 0, "{boundary}");
        let initialization = db.lane_initialization("io-lane").unwrap();
        if boundary == "association_sqlite_commit" {
            let initialization = initialization.unwrap();
            assert_eq!(initialization.phase, LaneInitializationPhase::Materialized);
            assert_eq!(owners_for_lane(&sqlite, "io-lane"), 0);
            assert!(temp.path().join(".trail/worktrees/io-lane").is_dir());
        } else {
            assert!(
                initialization
                    .as_ref()
                    .is_none_or(|record| record.phase == LaneInitializationPhase::Reserved),
                "boundary {boundary}: {initialization:?}"
            );
            assert!(!temp.path().join(".trail/worktrees/io-lane").exists());
        }
    }
}

#[test]
fn post_association_failure_is_durable_committed_repair_and_repairs_once() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let mut db = Trail::open(temp.path()).unwrap();
    trail::test_support::set_lane_association_failure_for_current_thread(Some(
        "spawn_after_commit",
    ));
    let error = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "repair-lane",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap_err();
    trail::test_support::set_lane_association_failure_for_current_thread(None);
    match error {
        trail::Error::CommittedRepairRequired {
            lane,
            phase,
            committed,
            repair,
            ..
        } => {
            assert_eq!(lane, "repair-lane");
            assert_eq!(phase, LaneInitializationPhase::RepairRequired);
            assert!(committed);
            assert_eq!(repair, "trail lane repair-initialization repair-lane");
        }
        other => panic!("expected committed repair, got {other:?}"),
    }
    let stored = db.lane_initialization("repair-lane").unwrap().unwrap();
    assert_eq!(stored.phase, LaneInitializationPhase::RepairRequired);
    assert_eq!(
        stored.repair_command.as_deref(),
        Some("trail lane repair-initialization repair-lane")
    );
    let repaired = db.repair_lane_initialization("repair-lane").unwrap();
    assert_eq!(repaired.phase, LaneInitializationPhase::ObserverReady);
    assert_eq!(
        spawn_events_for_lane(
            &temp.path().join(".trail/index/trail.sqlite"),
            "repair-lane"
        ),
        1
    );
}

#[test]
fn terminal_transition_and_owner_release_are_one_transaction() {
    let successful = tempfile::tempdir().unwrap();
    initialize_workspace(successful.path());
    let successful_sqlite = successful.path().join(".trail/index/trail.sqlite");
    let mut successful_db = Trail::open(successful.path()).unwrap();
    let report = successful_db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "terminal-success",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    assert_eq!(report.phase, LaneInitializationPhase::ObserverReady);
    assert_eq!(owners_for_lane(&successful_sqlite, "terminal-success"), 0);

    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let mut db = Trail::open(temp.path()).unwrap();
    reject_lane_initialization_owner_deletes(&sqlite);
    let error = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "terminal-release",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap_err();

    match error {
        trail::Error::CommittedRepairRequired {
            committed: true,
            phase,
            ..
        } => assert_eq!(phase, LaneInitializationPhase::Associated),
        other => panic!("terminal owner-release failure escaped as {other:?}"),
    }
    assert_eq!(
        db.lane_initialization("terminal-release")
            .unwrap()
            .unwrap()
            .phase,
        LaneInitializationPhase::Associated
    );
    assert_eq!(owners_for_lane(&sqlite, "terminal-release"), 1);
    assert_eq!(spawn_events_for_lane(&sqlite, "terminal-release"), 1);
}

#[test]
fn repair_state_persistence_failure_preserves_committed_outcome_contract() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let mut db = Trail::open(temp.path()).unwrap();
    reject_lane_initialization_phases(&sqlite, &["repair_required"]);
    trail::test_support::set_lane_association_failure_for_current_thread(Some("spawn_event"));
    let error = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "repair-write-failure",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap_err();
    trail::test_support::set_lane_association_failure_for_current_thread(None);

    let durable_phase = db
        .lane_initialization("repair-write-failure")
        .unwrap()
        .unwrap()
        .phase;
    match error {
        trail::Error::CommittedRepairRequired {
            committed: true,
            phase,
            ..
        } => assert_eq!(phase, durable_phase),
        other => panic!("post-commit failure escaped as {other:?}"),
    }
    assert_eq!(durable_phase, LaneInitializationPhase::Associated);
    assert_eq!(owners_for_lane(&sqlite, "repair-write-failure"), 0);
}

#[test]
fn observer_ready_and_repair_persistence_failures_preserve_committed_outcome_contract() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let mut db = Trail::open(temp.path()).unwrap();
    reject_lane_initialization_phases(&sqlite, &["observer_ready", "repair_required"]);
    reject_lane_initialization_owner_deletes(&sqlite);
    let error = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "observer-write-failure",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap_err();

    let durable_phase = db
        .lane_initialization("observer-write-failure")
        .unwrap()
        .unwrap()
        .phase;
    match error {
        trail::Error::CommittedRepairRequired {
            committed: true,
            phase,
            ..
        } => assert_eq!(phase, durable_phase),
        other => panic!("post-commit failure escaped as {other:?}"),
    }
    assert_eq!(durable_phase, LaneInitializationPhase::Associated);
    assert_eq!(spawn_events_for_lane(&sqlite, "observer-write-failure"), 1);
    assert_eq!(owners_for_lane(&sqlite, "observer-write-failure"), 1);
}

#[test]
fn concurrent_identical_spawn_requests_replay_one_committed_result() {
    struct AuthorityOverride;
    impl Drop for AuthorityOverride {
        fn drop(&mut self) {
            trail::test_support::set_changed_path_authority_override(false);
        }
    }

    const CALLERS: usize = 16;
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let workspace = Arc::new(temp.path().to_path_buf());
    let start = Arc::new(Barrier::new(CALLERS + 1));
    let handles = (0..CALLERS)
        .map(|_| {
            let workspace = Arc::clone(&workspace);
            let start = Arc::clone(&start);
            thread::spawn(move || {
                trail::test_support::set_changed_path_authority_override(true);
                let _authority = AuthorityOverride;
                let mut db = Trail::open(&*workspace).unwrap();
                start.wait();
                db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                    "duplicate-delivery",
                    Some("main"),
                    LaneWorkdirMode::Virtual,
                    Some("codex".into()),
                    Some("gpt-5".into()),
                    None,
                    &[],
                    false,
                )
            })
        })
        .collect::<Vec<_>>();
    start.wait();

    let reports = handles
        .into_iter()
        .map(|handle| handle.join().unwrap())
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(reports
        .iter()
        .all(|report| report.phase == LaneInitializationPhase::ObserverReady));
    assert!(reports
        .iter()
        .all(|report| report.initialization_id == reports[0].initialization_id));
    assert!(reports
        .iter()
        .all(|report| report.lane_id == reports[0].lane_id));

    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    assert_eq!(rows_for_lane(&sqlite, "duplicate-delivery"), 1);
    assert_eq!(refs_for_lane(&sqlite, "duplicate-delivery"), 1);
    assert_eq!(spawn_events_for_lane(&sqlite, "duplicate-delivery"), 1);
}

#[test]
fn concurrent_identical_processes_replay_one_committed_result() {
    const CALLERS: usize = 16;
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let start = temp.path().join("processes.start");
    let readiness = (0..CALLERS)
        .map(|index| temp.path().join(format!("process-{index:02}.ready")))
        .collect::<Vec<_>>();
    let mut children = Vec::with_capacity(CALLERS);
    for ready in &readiness {
        let mut child = initialization_child(temp.path())
            .env(CHILD_EXPECT_SUCCESS, "1")
            .env(CHILD_READY, ready)
            .env(CHILD_START, &start)
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() && Instant::now() < deadline {
            assert!(
                child.try_wait().unwrap().is_none(),
                "child exited while opening Trail"
            );
            thread::sleep(Duration::from_millis(5));
        }
        assert!(
            ready.exists(),
            "child did not open Trail before the deadline"
        );
        children.push(child);
    }
    fs::write(&start, b"start\n").unwrap();
    for child in children.drain(..) {
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "identical spawn child failed: {}; stderr: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    assert_eq!(rows_for_lane(&sqlite, "crash-lane"), 1);
    assert_eq!(refs_for_lane(&sqlite, "crash-lane"), 1);
    assert_eq!(spawn_events_for_lane(&sqlite, "crash-lane"), 1);
    assert_eq!(owners_for_lane(&sqlite, "crash-lane"), 0);
    let phase: String = Connection::open(&sqlite)
        .unwrap()
        .query_row(
            "SELECT phase FROM lane_initializations WHERE lane_name='crash-lane'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(phase, "observer_ready");
    assert!(lane_initialization_filesystem_artifacts(&temp.path().join(".trail")).is_empty());
    assert!(
        !temp.path().join(".trail/lock").exists(),
        "authority-off SQLite contention created or retained the workspace lock"
    );
}

#[test]
fn sixty_four_unrelated_initializations_reach_observer_ready_without_owners() {
    struct AuthorityOverride;
    impl Drop for AuthorityOverride {
        fn drop(&mut self) {
            trail::test_support::set_changed_path_authority_override(false);
        }
    }

    const OBSERVERS: usize = 64;
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let _authority = AuthorityOverride;
    let workspace = Arc::new(temp.path().to_path_buf());
    let start = Arc::new(Barrier::new(OBSERVERS + 1));
    let handles = (0..OBSERVERS)
        .map(|index| {
            let workspace = Arc::clone(&workspace);
            let start = Arc::clone(&start);
            thread::spawn(move || {
                struct WorkerAuthorityOverride;
                impl Drop for WorkerAuthorityOverride {
                    fn drop(&mut self) {
                        trail::test_support::set_changed_path_authority_override(false);
                    }
                }
                trail::test_support::set_changed_path_authority_override(true);
                let _authority = WorkerAuthorityOverride;
                let lane = format!("observer-{index:02}");
                let mut db = Trail::open(&*workspace).unwrap();
                start.wait();
                let result = db.spawn_lane_with_workdir_mode_paths_and_neighbors(
                    &lane,
                    Some("main"),
                    LaneWorkdirMode::PortableCopy,
                    None,
                    None,
                    None,
                    &[],
                    false,
                );
                (lane, result)
            })
        })
        .collect::<Vec<_>>();
    start.wait();

    for handle in handles {
        let (lane, result) = handle.join().unwrap();
        match result {
            Ok(report) => assert_eq!(
                report.phase,
                LaneInitializationPhase::ObserverReady,
                "{lane} did not reach observer_ready"
            ),
            Err(error) => {
                assert_ne!(error.code(), "DAEMON_UNAVAILABLE", "{lane}: {error}");
                panic!("{lane} startup failed ambiguously: {error:?}");
            }
        }
    }

    let db_dir = temp.path().join(".trail");
    let sqlite = db_dir.join("index/trail.sqlite");
    let ready: i64 = Connection::open(&sqlite)
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM lane_initializations WHERE phase='observer_ready'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(ready, OBSERVERS as i64);
    assert_eq!(active_owner_rows(&sqlite), 0);
    assert!(lane_initialization_filesystem_artifacts(&db_dir).is_empty());
    assert!(!db_dir.join("lock").exists(), "live workspace lock leaked");
    assert!(
        fs::read_dir(&db_dir).unwrap().all(|entry| {
            !entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".workspace-lock-candidate-")
        }),
        "workspace lock candidate leaked"
    );
}

#[test]
fn http_committed_repair_is_409_with_explicit_committed_body() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let mut db = Trail::open(temp.path()).unwrap();
    trail::test_support::set_lane_association_failure_for_current_thread(Some(
        "spawn_after_commit",
    ));
    let body = br#"{"name":"http-repair","from":"main","workdir_mode":"virtual"}"#;
    let request = format!(
        "POST /v1/lanes HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    )
    .into_bytes()
    .into_iter()
    .chain(body.iter().copied())
    .collect::<Vec<_>>();
    let response = trail::server::handle_http_request(&mut db, &request);
    trail::test_support::set_lane_association_failure_for_current_thread(None);
    assert_eq!(response.status, 409);
    let value: serde_json::Value = response.body_json().unwrap();
    assert_eq!(value["error"]["code"], "COMMITTED_REPAIR_REQUIRED");
    assert_eq!(value["error"]["status"], 409);
    assert_eq!(value["error"]["details"]["lane"], "http-repair");
    assert_eq!(value["error"]["details"]["phase"], "repair_required");
    assert_eq!(value["error"]["details"]["committed"], true);
    assert_eq!(
        value["error"]["details"]["repair"],
        "trail lane repair-initialization http-repair"
    );
}

#[allow(dead_code)]
fn fsynced_handshake(path: &PathBuf) {
    let mut file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(path)
        .unwrap();
    file.write_all(b"durable\n").unwrap();
    file.sync_all().unwrap();
    FileSync::sync_parent(path);
}

struct FileSync;

impl FileSync {
    fn sync_parent(path: &Path) {
        OpenOptions::new()
            .read(true)
            .open(path.parent().unwrap())
            .unwrap()
            .sync_all()
            .unwrap();
    }
}
