use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{Duration, Instant};

use rusqlite::Connection;
use trail::{Actor, InitImportMode, LaneInitializationPhase, LaneWorkdirMode, Trail};

const CHILD_WORKSPACE: &str = "TRAIL_TEST_INITIALIZATION_CHILD_WORKSPACE";
const CRASH_AFTER: &str = "TRAIL_TEST_LANE_INITIALIZATION_CRASH_AFTER";
const HANDSHAKE: &str = "TRAIL_TEST_LANE_INITIALIZATION_HANDSHAKE";

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

#[test]
fn lane_initialization_crash_child() {
    let Ok(workspace) = std::env::var(CHILD_WORKSPACE) else {
        return;
    };
    let mut db = Trail::open(workspace).unwrap();
    let _ = db.spawn_lane_with_workdir_mode_paths_and_neighbors(
        "crash-lane",
        Some("main"),
        LaneWorkdirMode::PortableCopy,
        Some("codex".into()),
        Some("gpt-5".into()),
        None,
        &[],
        false,
    );
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
        assert!(
            initialization
                .as_ref()
                .is_none_or(|record| record.phase == LaneInitializationPhase::Reserved),
            "boundary {boundary}: {initialization:?}"
        );
        assert!(!temp.path().join(".trail/worktrees/io-lane").exists());
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

    assert!(
        matches!(
            error,
            trail::Error::CommittedRepairRequired {
                committed: true,
                ..
            }
        ),
        "post-commit failure escaped as {error:?}"
    );
    assert_eq!(
        db.lane_initialization("repair-write-failure")
            .unwrap()
            .unwrap()
            .phase,
        LaneInitializationPhase::Associated
    );
}

#[test]
fn observer_ready_and_repair_persistence_failures_preserve_committed_outcome_contract() {
    let temp = tempfile::tempdir().unwrap();
    initialize_workspace(temp.path());
    let sqlite = temp.path().join(".trail/index/trail.sqlite");
    let mut db = Trail::open(temp.path()).unwrap();
    reject_lane_initialization_phases(&sqlite, &["observer_ready", "repair_required"]);
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

    assert!(
        matches!(
            error,
            trail::Error::CommittedRepairRequired {
                committed: true,
                ..
            }
        ),
        "post-commit failure escaped as {error:?}"
    );
    assert_eq!(
        db.lane_initialization("observer-write-failure")
            .unwrap()
            .unwrap()
            .phase,
        LaneInitializationPhase::Associated
    );
    assert_eq!(spawn_events_for_lane(&sqlite, "observer-write-failure"), 1);
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
fn sixty_four_observers_serialize_publication_without_ambiguous_failures() {
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
