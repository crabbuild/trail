use std::fs;
use std::path::{Path, PathBuf};

use rusqlite::{types::Value as SqlValue, Connection};
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

    fn sqlite_path(&self) -> PathBuf {
        self.workspace().join(".trail/index/trail.sqlite")
    }
}

fn api_request(method: &str, path: &str, body: serde_json::Value) -> Vec<u8> {
    let body = serde_json::to_vec(&body).unwrap();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
        body.len()
    );
    request.into_bytes().into_iter().chain(body).collect()
}

fn durable_lane_state(sqlite_path: &Path) -> Vec<(String, Vec<Vec<SqlValue>>)> {
    let conn = Connection::open(sqlite_path).unwrap();
    [
        "lane_initializations",
        "refs",
        "lanes",
        "lane_branches",
        "lane_events",
    ]
    .into_iter()
    .map(|table| {
        let mut stmt = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let column_count = stmt.column_count();
        let rows = stmt
            .query_map([], |row| {
                (0..column_count)
                    .map(|index| row.get(index))
                    .collect::<rusqlite::Result<Vec<SqlValue>>>()
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap();
        (table.to_string(), rows)
    })
    .collect()
}

fn filesystem_state(workspace: &Path) -> Vec<(PathBuf, String, Vec<u8>)> {
    fn visit(root: &Path, path: &Path, entries: &mut Vec<(PathBuf, String, Vec<u8>)>) {
        if !path.exists() {
            return;
        }
        let mut children = fs::read_dir(path)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        children.sort();
        for child in children {
            let relative = child.strip_prefix(root).unwrap().to_path_buf();
            let metadata = fs::symlink_metadata(&child).unwrap();
            if metadata.file_type().is_symlink() {
                entries.push((
                    relative,
                    "symlink".into(),
                    fs::read_link(&child)
                        .unwrap()
                        .to_string_lossy()
                        .as_bytes()
                        .to_vec(),
                ));
            } else if metadata.is_dir() {
                entries.push((relative, "directory".into(), Vec::new()));
                visit(root, &child, entries);
            } else {
                entries.push((relative, "file".into(), fs::read(&child).unwrap()));
            }
        }
    }

    let mut entries = Vec::new();
    for path in [".trail/refs", ".trail/worktrees"] {
        visit(workspace, &workspace.join(path), &mut entries);
    }
    entries
}

fn ref_operation(sqlite_path: &Path, name: &str) -> String {
    Connection::open(sqlite_path)
        .unwrap()
        .query_row(
            "SELECT operation_id FROM refs WHERE name=?1",
            [name],
            |row| row.get(0),
        )
        .unwrap()
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
fn http_duplicate_spawn_conflict_returns_shared_identity_details() {
    let mut fixture = LaneInitializationFixture::new();
    let first = trail::server::handle_http_request(
        fixture.db_mut(),
        &api_request(
            "POST",
            "/v1/lanes",
            serde_json::json!({
                "name": "http-conflict",
                "from": "main",
                "workdir_mode": "virtual",
                "provider": "codex",
                "model": "gpt-5"
            }),
        ),
    );
    assert_eq!(first.status, 201);
    let first: serde_json::Value = first.body_json().unwrap();

    let conflict = trail::server::handle_http_request(
        fixture.db_mut(),
        &api_request(
            "POST",
            "/v1/lanes",
            serde_json::json!({
                "name": "http-conflict",
                "from": "main",
                "workdir_mode": "virtual",
                "provider": "codex",
                "model": "different-model"
            }),
        ),
    );
    let status = conflict.status;
    let conflict: serde_json::Value = conflict.body_json().unwrap();

    assert_eq!(status, 409, "HTTP response: {conflict}");
    assert_eq!(conflict["error"]["code"], "LANE_INITIALIZATION_CONFLICT");
    assert_eq!(conflict["error"]["status"], 409);
    assert_eq!(conflict["error"]["exit"], 2);
    assert_eq!(conflict["error"]["details"]["lane"], "http-conflict");
    assert_eq!(
        conflict["error"]["details"]["existing_fingerprint"],
        first["request_fingerprint"]
    );
    assert_ne!(
        conflict["error"]["details"]["requested_fingerprint"],
        first["request_fingerprint"]
    );
}

#[test]
fn mcp_duplicate_spawn_conflict_returns_shared_identity_details() {
    let mut fixture = LaneInitializationFixture::new();
    let call = |id, model| {
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": "trail.lane_spawn",
                "arguments": {
                    "name": "mcp-conflict",
                    "from_ref": "main",
                    "workdir_mode": "virtual",
                    "provider": "codex",
                    "model": model
                }
            }
        })
    };
    let first = trail::mcp::handle_json_rpc(fixture.db_mut(), call(1, "gpt-5")).unwrap();
    assert_eq!(first["result"]["isError"], false, "MCP response: {first}");
    let first_fingerprint = first["result"]["structuredContent"]["request_fingerprint"]
        .as_str()
        .unwrap();

    let conflict =
        trail::mcp::handle_json_rpc(fixture.db_mut(), call(2, "different-model")).unwrap();
    let structured = &conflict["result"]["structuredContent"]["error"];

    assert_eq!(conflict["result"]["isError"], true);
    assert_eq!(structured["code"], "LANE_INITIALIZATION_CONFLICT");
    assert_eq!(structured["status"], 409);
    assert_eq!(structured["exit"], 2);
    assert_eq!(structured["details"]["lane"], "mcp-conflict");
    assert_eq!(
        structured["details"]["existing_fingerprint"],
        first_fingerprint
    );
    assert_ne!(
        structured["details"]["requested_fingerprint"],
        first_fingerprint
    );
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
fn different_sparse_paths_conflict_with_the_reserved_identity() {
    let mut fixture = LaneInitializationFixture::new();
    let first = fixture
        .db_mut()
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "sparse-conflict",
            Some("main"),
            LaneWorkdirMode::Sparse,
            None,
            None,
            None,
            &["a".into()],
            false,
        )
        .unwrap();
    let error = fixture
        .db_mut()
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "sparse-conflict",
            Some("main"),
            LaneWorkdirMode::Sparse,
            None,
            None,
            None,
            &["b".into()],
            false,
        )
        .unwrap_err();

    assert_initialization_conflict(error, "sparse-conflict", &first.request_fingerprint);
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
fn different_explicit_destinations_conflict_with_the_reserved_identity() {
    let mut fixture = LaneInitializationFixture::new();
    let first_destination = PathBuf::from(".trail/worktrees/destination-a");
    let second_destination = PathBuf::from(".trail/worktrees/destination-b");
    let first = fixture
        .db_mut()
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "destination-conflict",
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            None,
            None,
            Some(first_destination),
            &[],
            false,
        )
        .unwrap();
    let error = fixture
        .db_mut()
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "destination-conflict",
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            None,
            None,
            Some(second_destination.clone()),
            &[],
            false,
        )
        .unwrap_err();

    assert_initialization_conflict(error, "destination-conflict", &first.request_fingerprint);
    assert!(!fixture.workspace().join(second_destination).exists());
}

#[test]
fn raw_change_source_is_a_stable_detached_identity_distinct_from_its_branch() {
    let mut fixture = LaneInitializationFixture::new();
    let main_change = fixture
        .db_mut()
        .list_branches()
        .unwrap()
        .into_iter()
        .find(|branch| branch.name == "main")
        .unwrap()
        .change_id
        .0;
    let first = spawn_virtual(
        &mut fixture,
        "detached-source",
        Some(&main_change),
        None,
        None,
        false,
    )
    .unwrap();
    let replay = spawn_virtual(
        &mut fixture,
        "detached-source",
        Some(&main_change),
        None,
        None,
        false,
    )
    .unwrap();

    assert_eq!(replay.initialization_id, first.initialization_id);
    assert_eq!(replay.request_fingerprint, first.request_fingerprint);
    assert!(replay.resumed);

    let error = spawn_virtual(
        &mut fixture,
        "detached-source",
        Some("main"),
        None,
        None,
        false,
    )
    .unwrap_err();
    assert_initialization_conflict(error, "detached-source", &first.request_fingerprint);
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
    let durable_before = durable_lane_state(&fixture.sqlite_path());
    let filesystem_before = filesystem_state(fixture.workspace());

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
    assert_eq!(durable_lane_state(&fixture.sqlite_path()), durable_before);
    assert_eq!(filesystem_state(fixture.workspace()), filesystem_before);
}

#[test]
fn lane_ref_without_initialization_row_fails_stably_without_reserving_or_repairing() {
    let mut fixture = LaneInitializationFixture::new();
    spawn_virtual(
        &mut fixture,
        "orphaned-initialization",
        Some("main"),
        None,
        None,
        false,
    )
    .unwrap();
    Connection::open(fixture.sqlite_path())
        .unwrap()
        .execute(
            "DELETE FROM lane_initializations WHERE lane_name=?1",
            ["orphaned-initialization"],
        )
        .unwrap();
    assert!(fixture
        .db_mut()
        .lane_initialization("orphaned-initialization")
        .unwrap()
        .is_none());
    let durable_before = durable_lane_state(&fixture.sqlite_path());
    let filesystem_before = filesystem_state(fixture.workspace());

    for _ in 0..2 {
        let error = spawn_virtual(
            &mut fixture,
            "orphaned-initialization",
            Some("main"),
            None,
            None,
            false,
        )
        .unwrap_err();
        assert_eq!(error.code(), "INVALID_INPUT");
        assert_eq!(error.exit_code(), 2);
        assert_eq!(
            error.to_string(),
            "invalid input: lane `orphaned-initialization` already exists without initialization identity"
        );
        assert!(fixture
            .db_mut()
            .lane_initialization("orphaned-initialization")
            .unwrap()
            .is_none());
        assert_eq!(durable_lane_state(&fixture.sqlite_path()), durable_before);
        assert_eq!(filesystem_state(fixture.workspace()), filesystem_before);
    }
}

#[test]
fn virtual_retains_source_operation_while_materialized_replaces_it() {
    let mut virtual_fixture = LaneInitializationFixture::new();
    let source_operation = ref_operation(&virtual_fixture.sqlite_path(), "refs/branches/main");
    spawn_virtual(
        &mut virtual_fixture,
        "virtual-operation",
        Some("main"),
        None,
        None,
        false,
    )
    .unwrap();
    let virtual_initialization = virtual_fixture
        .db_mut()
        .lane_initialization("virtual-operation")
        .unwrap()
        .unwrap();
    assert_eq!(virtual_initialization.operation_id, source_operation);

    let mut materialized_fixture = LaneInitializationFixture::new();
    let source_operation = ref_operation(&materialized_fixture.sqlite_path(), "refs/branches/main");
    materialized_fixture
        .db_mut()
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "materialized-operation",
            Some("main"),
            LaneWorkdirMode::PortableCopy,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    let materialized_initialization = materialized_fixture
        .db_mut()
        .lane_initialization("materialized-operation")
        .unwrap()
        .unwrap();
    assert_ne!(materialized_initialization.operation_id, source_operation);
    assert!(!materialized_initialization.operation_id.is_empty());
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
