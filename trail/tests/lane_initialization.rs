use std::fs;
use std::path::{Path, PathBuf};

use trail::{Actor, Error, InitImportMode, LaneWorkdirMode, Trail};

struct LaneInitializationFixture {
    temp: tempfile::TempDir,
    db: Trail,
}

impl LaneInitializationFixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        fs::create_dir_all(temp.path().join("a")).unwrap();
        fs::create_dir_all(temp.path().join("b")).unwrap();
        fs::write(temp.path().join("a/file.txt"), "a\n").unwrap();
        fs::write(temp.path().join("b/file.txt"), "b\n").unwrap();
        let mut db = Trail::open(temp.path()).unwrap();
        db.record(
            Some("main"),
            Some("record initialization fixture".into()),
            Actor::human(),
            false,
        )
        .unwrap();
        Self { temp, db }
    }

    fn workspace(&self) -> &Path {
        self.temp.path()
    }

    fn db_mut(&mut self) -> &mut Trail {
        &mut self.db
    }
}

fn spawn_virtual(
    fixture: &mut LaneInitializationFixture,
    name: &str,
    from: Option<&str>,
    provider: Option<&str>,
    model: Option<&str>,
    include_neighbors: bool,
) -> trail::Result<trail::LaneSpawnReport> {
    fixture
        .db_mut()
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            name,
            from,
            LaneWorkdirMode::Virtual,
            provider.map(str::to_owned),
            model.map(str::to_owned),
            None,
            &[],
            include_neighbors,
        )
}

fn assert_initialization_conflict(error: Error, lane: &str, existing_fingerprint: &str) -> String {
    assert_eq!(error.code(), "LANE_INITIALIZATION_CONFLICT");
    assert_eq!(error.exit_code(), 2);
    match error {
        Error::LaneInitializationConflict {
            lane: actual_lane,
            existing_fingerprint: actual_existing,
            requested_fingerprint,
        } => {
            assert_eq!(actual_lane, lane);
            assert_eq!(actual_existing, existing_fingerprint);
            assert_ne!(requested_fingerprint, existing_fingerprint);
            requested_fingerprint
        }
        other => panic!("expected lane initialization conflict, got {other:?}"),
    }
}

#[test]
fn equivalent_spawn_requests_share_one_canonical_initialization_identity() {
    let mut fixture = LaneInitializationFixture::new();
    let first = fixture
        .db_mut()
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "agent-1",
            Some("main"),
            LaneWorkdirMode::Sparse,
            Some("codex".into()),
            Some("gpt-5".into()),
            None,
            &["b".into(), "a".into(), "a".into()],
            true,
        )
        .unwrap();
    let replay = fixture
        .db_mut()
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "agent-1",
            Some("refs/heads/main"),
            LaneWorkdirMode::Sparse,
            Some("codex".into()),
            Some("gpt-5".into()),
            None,
            &["a".into(), "b".into()],
            true,
        )
        .unwrap();

    assert_eq!(replay.initialization_id, first.initialization_id);
    assert_eq!(replay.request_fingerprint, first.request_fingerprint);
    assert!(first.request_fingerprint.starts_with("sha256:"));
    assert!(replay.resumed);
    assert!(replay.committed);
}

#[test]
fn relative_and_canonical_destinations_share_one_canonical_identity() {
    let mut fixture = LaneInitializationFixture::new();
    let relative = PathBuf::from(".trail/worktrees/canonical-destination");
    let canonical = fixture.workspace().join(&relative);
    let first = fixture
        .db_mut()
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "destination-agent",
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            None,
            None,
            Some(relative),
            &[],
            false,
        )
        .unwrap();
    let replay = fixture
        .db_mut()
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "destination-agent",
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            None,
            None,
            Some(canonical),
            &[],
            false,
        )
        .unwrap();

    assert_eq!(replay.initialization_id, first.initialization_id);
    assert_eq!(replay.request_fingerprint, first.request_fingerprint);
    assert!(replay.resumed);
}

#[test]
fn same_fingerprint_replay_returns_the_canonical_ready_lane() {
    let mut fixture = LaneInitializationFixture::new();
    let first = spawn_virtual(
        &mut fixture,
        "replay-agent",
        Some("main"),
        Some("codex"),
        Some("gpt-5"),
        false,
    )
    .unwrap();
    let replay = spawn_virtual(
        &mut fixture,
        "replay-agent",
        Some("refs/heads/main"),
        Some("codex"),
        Some("gpt-5"),
        false,
    )
    .unwrap();

    assert_eq!(replay.lane_id, first.lane_id);
    assert_eq!(replay.initialization_id, first.initialization_id);
    assert_eq!(replay.request_fingerprint, first.request_fingerprint);
    assert_eq!(replay.phase, trail::LaneInitializationPhase::ObserverReady);
    assert!(replay.committed);
    assert!(replay.resumed);
}

#[test]
fn different_source_conflict_is_stable_and_mutation_free() {
    let mut fixture = LaneInitializationFixture::new();
    fixture
        .db_mut()
        .create_branch("older", Some("main"))
        .unwrap();
    fs::write(fixture.workspace().join("a/file.txt"), "new main\n").unwrap();
    fixture
        .db_mut()
        .record(
            Some("main"),
            Some("advance main".into()),
            Actor::human(),
            false,
        )
        .unwrap();
    let first = spawn_virtual(
        &mut fixture,
        "source-conflict",
        Some("older"),
        None,
        None,
        false,
    )
    .unwrap();
    let before = fixture
        .db_mut()
        .lane_initialization("source-conflict")
        .unwrap()
        .unwrap();

    let error = spawn_virtual(
        &mut fixture,
        "source-conflict",
        Some("main"),
        None,
        None,
        false,
    )
    .unwrap_err();
    assert_initialization_conflict(error, "source-conflict", &first.request_fingerprint);

    let after = fixture
        .db_mut()
        .lane_initialization("source-conflict")
        .unwrap()
        .unwrap();
    assert_eq!(after, before);
}

#[test]
fn different_mode_conflict_preserves_the_reserved_identity() {
    let mut fixture = LaneInitializationFixture::new();
    let first = spawn_virtual(
        &mut fixture,
        "mode-conflict",
        Some("main"),
        None,
        None,
        false,
    )
    .unwrap();
    let error = fixture
        .db_mut()
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "mode-conflict",
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap_err();

    assert_initialization_conflict(error, "mode-conflict", &first.request_fingerprint);
}

#[test]
fn neighbor_provider_and_model_conflicts_are_distinct_and_stable() {
    for (name, provider, model, include_neighbors) in [
        ("neighbor-conflict", Some("codex"), Some("gpt-5"), true),
        (
            "provider-conflict",
            Some("different-provider"),
            Some("gpt-5"),
            false,
        ),
        (
            "model-conflict",
            Some("codex"),
            Some("different-model"),
            false,
        ),
    ] {
        let mut fixture = LaneInitializationFixture::new();
        let first = spawn_virtual(
            &mut fixture,
            name,
            Some("main"),
            Some("codex"),
            Some("gpt-5"),
            false,
        )
        .unwrap();
        let error = spawn_virtual(
            &mut fixture,
            name,
            Some("main"),
            provider,
            model,
            include_neighbors,
        )
        .unwrap_err();

        assert_initialization_conflict(error, name, &first.request_fingerprint);
    }
}
