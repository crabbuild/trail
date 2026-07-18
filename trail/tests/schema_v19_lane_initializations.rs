use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rusqlite::{params, Connection};
use sha2::{Digest, Sha256};
use trail::{InitImportMode, LaneInitializationPhase, Trail};

const EXPECTED_COLUMNS: &[&str] = &[
    "initialization_id",
    "lane_name",
    "lane_id",
    "request_fingerprint",
    "operation_id",
    "phase",
    "workdir",
    "materialization_json",
    "last_error_code",
    "last_error_message",
    "repair_command",
    "created_at",
    "updated_at",
];

struct Schema18Fixture {
    temp: tempfile::TempDir,
    db_path: PathBuf,
}

impl Schema18Fixture {
    fn clean() -> Self {
        let temp = tempfile::tempdir().unwrap();
        trail::test_support::create_schema_v18_fixture(temp.path()).unwrap();
        assert_eq!(sqlite_user_version(&Self::db_path_for(temp.path())), 18);
        let db_path = Self::db_path_for(temp.path());
        Self { temp, db_path }
    }

    fn with_clean_and_inconsistent_lanes() -> Self {
        let fixture = Self::clean();
        let conn = Connection::open(fixture.db_path()).unwrap();
        let (change_id, root_id, operation_id, generation): (String, String, String, i64) = conn
            .query_row(
                "SELECT change_id,root_id,operation_id,generation FROM refs
                 WHERE name='refs/branches/main'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();

        for (name, lane_id) in [("clean", "lane_clean"), ("missing-marker", "lane_missing")] {
            let ref_name = format!("refs/lanes/{name}");
            let workdir = fixture.workspace().join(".trail/worktrees").join(name);
            fs::create_dir_all(workdir.join(".trail")).unwrap();
            let workdir = workdir.to_string_lossy().to_string();
            let metadata = serde_json::json!({
                "requested_workdir_mode": "portable-copy",
                "workdir_mode": "portable-copy",
                "workdir_backend": "copy",
                "materialization": {
                    "cloned_files": 0,
                    "cloned_bytes": 0,
                    "copied_files": 1,
                    "copied_bytes": 7
                },
                "sparse_paths": [],
                "include_neighbors": false,
                "transparent_cow_available": false
            });
            conn.execute(
                "INSERT INTO refs(name,change_id,root_id,operation_id,generation,updated_at)
                 VALUES(?1,?2,?3,?4,?5,100)",
                params![ref_name, change_id, root_id, operation_id, generation],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO lanes(lane_id,name,kind,provider,model,created_at,metadata_json)
                 VALUES(?1,?2,'coding-lane','test-provider','test-model',100,?3)",
                params![lane_id, name, metadata.to_string()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO lane_branches(
                     lane_id,ref_name,base_change,head_change,base_root,head_root,
                     session_id,workdir,status,created_at,updated_at)
                 VALUES(?1,?2,?3,?3,?4,?4,NULL,?5,'active',100,100)",
                params![lane_id, ref_name, change_id, root_id, workdir],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO lane_events(
                     event_id,lane_id,turn_id,session_id,event_type,change_id,
                     message_id,payload_json,created_at)
                 VALUES(?1,?2,NULL,NULL,'lane_spawned',?3,NULL,?4,100)",
                params![
                    format!("event_{lane_id}"),
                    lane_id,
                    change_id,
                    serde_json::json!({
                        "ref_name": ref_name,
                        "base_root": root_id,
                        "workdir": workdir,
                        "requested_workdir_mode": "portable-copy",
                        "workdir_mode": "portable-copy",
                        "workdir_backend": "copy",
                        "materialization": metadata["materialization"],
                        "sparse_paths": [],
                        "include_neighbors": false,
                        "transparent_cow_available": false
                    })
                    .to_string()
                ],
            )
            .unwrap();
        }

        conn.execute(
            "INSERT INTO changed_path_scopes(
                 scope_id,scope_kind,owner_id,scope_root,scope_root_identity,
                 filesystem_identity,filesystem_kind,case_sensitive,ref_name,
                 ref_generation,change_id,baseline_root_id,policy_fingerprint,
                 policy_dependency_generation,trust_state,trust_reason,epoch,
                 provider_id,provider_identity,durable_cursor,linearizable_fence,
                 rename_pairing,overflow_scope,filesystem_supported,clean_proof_allowed,
                 power_loss_durability,durable_offset,folded_offset,observer_owner_token,
                 observer_heartbeat_at,created_at,updated_at)
             SELECT 'scope_clean','materialized_lane','lane_clean',?1,'root-identity',
                    'filesystem-identity','native',1,name,generation,change_id,root_id,
                    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',1,
                    'trusted','clean_checkpoint',1,'test-provider','provider-identity',1,1,
                    1,0,1,1,1,1,1,
                    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                    100,100,100
             FROM refs WHERE name='refs/lanes/clean'",
            [fixture
                .workspace()
                .join(".trail/worktrees/clean")
                .to_string_lossy()
                .to_string()],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO changed_path_observer_owners(
                 scope_id,epoch,owner_token,provider_id,provider_identity,lease_state,
                 fence_nonce,acquired_at,heartbeat_at,expires_at,updated_at)
             VALUES('scope_clean',1,
                    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                    'test-provider','provider-identity','active',zeroblob(32),1,100,4102444800,100)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO changed_path_observer_segments(
                 scope_id,epoch,segment_id,owner_token,provider_id,first_sequence,
                 last_sequence,durable_end_offset,folded_end_offset,segment_path,state,
                 created_at,updated_at)
             VALUES('scope_clean',1,'segment_clean',
                    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                    'test-provider',1,1,1,1,'segment_clean.cpl','sealed',100,100)",
            [],
        )
        .unwrap();
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| {
            row.get::<_, i64>(0)
        })
        .unwrap();
        drop(conn);
        fixture
    }

    fn workspace(&self) -> &Path {
        self.temp.path()
    }

    fn db_path_for(workspace: &Path) -> PathBuf {
        workspace.join(".trail/index/trail.sqlite")
    }

    fn db_path(&self) -> &Path {
        &self.db_path
    }

    fn database_image_hashes(&self) -> Vec<(String, String)> {
        ["", "-wal", "-journal"]
            .into_iter()
            .filter_map(|suffix| {
                let path = PathBuf::from(format!("{}{}", self.db_path().display(), suffix));
                path.is_file().then(|| {
                    let bytes = fs::read(&path).unwrap();
                    (
                        suffix.to_string(),
                        hex::encode(Sha256::digest(bytes.as_slice())),
                    )
                })
            })
            .collect()
    }
}

fn sqlite_user_version(path: &Path) -> i64 {
    Connection::open(path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap()
}

fn table_columns(path: &Path) -> Vec<String> {
    Connection::open(path)
        .unwrap()
        .prepare("SELECT name FROM pragma_table_info('lane_initializations') ORDER BY cid")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn fresh_schema_19_has_the_exact_lane_initialization_contract() {
    let temp = tempfile::tempdir().unwrap();
    Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
    let db_path = Schema18Fixture::db_path_for(temp.path());
    assert_eq!(sqlite_user_version(&db_path), 19);
    assert_eq!(
        table_columns(&db_path),
        EXPECTED_COLUMNS
            .iter()
            .map(|column| (*column).to_string())
            .collect::<Vec<_>>()
    );

    let conn = Connection::open(&db_path).unwrap();
    let unique_lane_name_indexes = conn
        .prepare(
            "SELECT idx.name
             FROM pragma_index_list('lane_initializations') idx
             WHERE idx.[unique]=1
               AND (SELECT group_concat(name, ',') FROM pragma_index_info(idx.name))='lane_name'",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(unique_lane_name_indexes.len(), 1);
    let invalid = conn.execute(
        "INSERT INTO lane_initializations(
             initialization_id,lane_name,lane_id,request_fingerprint,operation_id,
             phase,created_at,updated_at)
         VALUES('invalid','invalid','lane_invalid','fingerprint','operation','ready',1,1)",
        [],
    );
    assert!(invalid.is_err());
}

#[test]
fn schema_18_migrates_once_and_backfills_every_existing_lane() {
    let fixture = Schema18Fixture::with_clean_and_inconsistent_lanes();
    let db = Trail::open(fixture.workspace()).unwrap();
    assert_eq!(sqlite_user_version(fixture.db_path()), 19);
    assert_eq!(
        db.lane_initialization("clean").unwrap().unwrap().phase,
        LaneInitializationPhase::ObserverReady
    );
    let repair = db.lane_initialization("missing-marker").unwrap().unwrap();
    assert_eq!(repair.phase, LaneInitializationPhase::RepairRequired);
    assert_eq!(
        repair.last_error_code.as_deref(),
        Some("LANE_INITIALIZATION_INCOMPLETE")
    );
    assert!(repair
        .last_error_message
        .as_deref()
        .unwrap()
        .contains("clean checkpoint"));
    assert_eq!(
        repair.repair_command.as_deref(),
        Some("trail lane repair-initialization missing-marker")
    );
    drop(db);
    drop(Trail::open(fixture.workspace()).unwrap());
    assert_eq!(sqlite_user_version(fixture.db_path()), 19);
    assert_eq!(
        Connection::open(fixture.db_path())
            .unwrap()
            .query_row("SELECT COUNT(*) FROM lane_initializations", [], |row| row
                .get::<_, i64>(
                0
            ))
            .unwrap(),
        2
    );
}

#[test]
fn failed_schema_18_migration_is_byte_invariant_and_retriable() {
    let fixture = Schema18Fixture::clean();
    let before = fixture.database_image_hashes();
    trail::test_support::fail_schema_v19_migration_after_ddl(fixture.db_path());
    assert!(Trail::open(fixture.workspace()).is_err());
    assert_eq!(fixture.database_image_hashes(), before);
    trail::test_support::clear_schema_v19_migration_failure(fixture.db_path());
    drop(Trail::open(fixture.workspace()).unwrap());
    assert_eq!(sqlite_user_version(fixture.db_path()), 19);
}

#[test]
fn corrupt_predecessor_future_and_partial_current_shapes_are_refused_without_mutation() {
    let partial_v18 = Schema18Fixture::clean();
    let conn = Connection::open(partial_v18.db_path()).unwrap();
    conn.execute_batch("PRAGMA foreign_keys=OFF; DROP TABLE lanes;")
        .unwrap();
    drop(conn);
    let before = partial_v18.database_image_hashes();
    let error = open_error(partial_v18.workspace());
    assert_eq!(error.code(), "SCHEMA_REINITIALIZE_REQUIRED");
    assert_eq!(partial_v18.database_image_hashes(), before);

    let future = Schema18Fixture::clean();
    let conn = Connection::open(future.db_path()).unwrap();
    conn.pragma_update(None, "user_version", 20).unwrap();
    conn.execute(
        "UPDATE schema_meta SET value='20' WHERE key='schema.version'",
        [],
    )
    .unwrap();
    drop(conn);
    let before = future.database_image_hashes();
    let error = open_error(future.workspace());
    assert_eq!(error.code(), "SCHEMA_REINITIALIZE_REQUIRED");
    assert!(error.to_string().contains("found version 20"));
    assert_eq!(future.database_image_hashes(), before);

    let partial_v19 = Schema18Fixture::clean();
    drop(Trail::open(partial_v19.workspace()).unwrap());
    let conn = Connection::open(partial_v19.db_path()).unwrap();
    conn.execute_batch("DROP TABLE lane_initializations;")
        .unwrap();
    drop(conn);
    let before = partial_v19.database_image_hashes();
    let error = open_error(partial_v19.workspace());
    assert_eq!(error.code(), "SCHEMA_REINITIALIZE_REQUIRED");
    assert_eq!(partial_v19.database_image_hashes(), before);
}

#[test]
fn schema_18_and_schema_19_backups_restore_through_schema_19() {
    let fixture = Schema18Fixture::clean();
    let archives = tempfile::tempdir().unwrap();
    let baseline = schema18_binary();
    let backup_v18 = archives.path().join("backup-v18");
    let output = Command::new(&baseline)
        .args(["--workspace", fixture.workspace().to_str().unwrap()])
        .args(["--json", "backup", "create"])
        .arg(&backup_v18)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "schema-18 backup failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let restored_v18 = archives.path().join("restored-v18");
    Trail::restore_backup(&restored_v18, &backup_v18, false).unwrap();
    assert_eq!(
        sqlite_user_version(&Schema18Fixture::db_path_for(&restored_v18)),
        19
    );

    let db = Trail::open(fixture.workspace()).unwrap();
    let backup_v19 = archives.path().join("backup-v19");
    db.create_backup(&backup_v19, false).unwrap();
    drop(db);
    let restored_v19 = archives.path().join("restored-v19");
    Trail::restore_backup(&restored_v19, &backup_v19, false).unwrap();
    assert_eq!(
        sqlite_user_version(&Schema18Fixture::db_path_for(&restored_v19)),
        19
    );
}

#[test]
fn schema_18_binary_refuses_a_migrated_schema_19_workspace() {
    let fixture = Schema18Fixture::clean();
    drop(Trail::open(fixture.workspace()).unwrap());
    let output = Command::new(schema18_binary())
        .args(["--workspace", fixture.workspace().to_str().unwrap()])
        .args(["--json", "status"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let rendered = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        rendered.contains("SCHEMA_REINITIALIZE_REQUIRED"),
        "{rendered}"
    );
    assert!(rendered.contains("found version 19"), "{rendered}");
}

fn schema18_binary() -> PathBuf {
    let binary = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join(".superpowers/sdd/trail-production-hardening/trail-schema18-baseline");
    let digest = hex::encode(Sha256::digest(fs::read(&binary).unwrap()));
    assert_eq!(
        digest,
        "dffa05662a8b68d69ba5f143e9aa7dd28c71a48c9fc3e778ba1cdccc4b301353"
    );
    binary
}

fn open_error(workspace: &Path) -> trail::Error {
    match Trail::open(workspace) {
        Ok(_) => panic!("incompatible schema opened"),
        Err(error) => error,
    }
}
