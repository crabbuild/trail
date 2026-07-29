use std::path::PathBuf;

use rusqlite::Connection;
use trail::test_support::SchemaV21MigrationBoundary;
use trail::{InitImportMode, LaneRetirementPhase, LaneWorkdirMode, Trail};

const EXPECTED_COLUMNS: &[(&str, &str, bool, bool)] = &[
    ("retirement_id", "TEXT", false, true),
    ("lane_id", "TEXT", true, false),
    ("former_name", "TEXT", true, false),
    ("kind", "TEXT", true, false),
    ("phase", "TEXT", true, false),
    ("resume_phase", "TEXT", false, false),
    ("forced", "INTEGER", true, false),
    ("provenance_json", "BLOB", true, false),
    ("private_paths_json", "BLOB", true, false),
    ("last_error_code", "TEXT", false, false),
    ("last_error_message", "TEXT", false, false),
    ("repair_command", "TEXT", false, false),
    ("created_at", "INTEGER", true, false),
    ("updated_at", "INTEGER", true, false),
    ("completed_at", "INTEGER", false, false),
];

struct SchemaV21Fixture {
    root: tempfile::TempDir,
    db_path: PathBuf,
}

impl SchemaV21Fixture {
    fn from_v20() -> Self {
        let root = tempfile::tempdir().unwrap();
        trail::test_support::create_schema_v20_fixture(root.path()).unwrap();
        let db_path = root.path().join(".trail/index/trail.sqlite");
        Self { root, db_path }
    }

    fn open_result(&self) -> trail::Result<Trail> {
        Trail::open(self.root.path())
    }

    fn raw_user_version(&self) -> i64 {
        Connection::open(&self.db_path)
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    fn table_exists(&self) -> bool {
        Connection::open(&self.db_path)
            .unwrap()
            .query_row(
                "SELECT EXISTS(
                     SELECT 1 FROM sqlite_master
                     WHERE type='table' AND name='lane_retirements'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap()
    }
}

fn retirement_columns(path: &std::path::Path) -> Vec<(String, String, bool, bool)> {
    Connection::open(path)
        .unwrap()
        .prepare(
            "SELECT name,type,[notnull],pk
             FROM pragma_table_info('lane_retirements')
             ORDER BY cid",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get::<_, i64>(2)? != 0,
                row.get::<_, i64>(3)? != 0,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

#[test]
fn v20_open_migrates_lane_retirement_journal_atomically_to_v21() {
    let fixture = SchemaV21Fixture::from_v20();
    let db = fixture.open_result().unwrap();
    assert_eq!(db.schema_user_version_for_test(), 21);
    let columns = retirement_columns(&fixture.db_path);
    let columns = columns
        .iter()
        .map(|(name, kind, not_null, primary_key)| {
            (name.as_str(), kind.as_str(), *not_null, *primary_key)
        })
        .collect::<Vec<_>>();
    assert_eq!(columns, EXPECTED_COLUMNS);

    let conn = Connection::open(&fixture.db_path).unwrap();
    assert!(conn
        .execute(
            "INSERT INTO lane_retirements(
                 retirement_id,lane_id,former_name,kind,phase,forced,
                 provenance_json,private_paths_json,created_at,updated_at)
             VALUES('ret_bad_kind','lane_a','a','archive','prepared',0,'{}','[]',1,1)",
            [],
        )
        .is_err());
    assert!(conn
        .execute(
            "INSERT INTO lane_retirements(
                 retirement_id,lane_id,former_name,kind,phase,forced,
                 provenance_json,private_paths_json,created_at,updated_at)
             VALUES('ret_bad_phase','lane_b','b','remove','unknown',0,'{}','[]',1,1)",
            [],
        )
        .is_err());
}

#[test]
fn fresh_init_creates_schema_v21_with_retirement_journal() {
    let root = tempfile::tempdir().unwrap();
    Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
    let db_path = root.path().join(".trail/index/trail.sqlite");
    let version: i64 = Connection::open(&db_path)
        .unwrap()
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert_eq!(version, 21);
    assert_eq!(retirement_columns(&db_path).len(), EXPECTED_COLUMNS.len());
}

#[test]
fn v21_migration_fault_rolls_back_table_metadata_and_user_version() {
    let fixture = SchemaV21Fixture::from_v20();
    trail::test_support::install_schema_v21_migration_failure(
        &fixture.db_path,
        SchemaV21MigrationBoundary::AfterDdlBeforeUserVersion,
    );
    assert!(fixture.open_result().is_err());
    assert_eq!(fixture.raw_user_version(), 20);
    assert!(!fixture.table_exists());
}

#[test]
fn schema_v21_backup_restore_preserves_completed_retirement_provenance() {
    let source = tempfile::tempdir().unwrap();
    Trail::init(source.path(), "main", InitImportMode::Empty, false).unwrap();
    let mut db = Trail::open(source.path()).unwrap();
    let spawned = db
        .spawn_lane_with_workdir_mode_paths_and_neighbors(
            "retired-before-backup",
            Some("main"),
            LaneWorkdirMode::Virtual,
            None,
            None,
            None,
            &[],
            false,
        )
        .unwrap();
    db.remove_lane("retired-before-backup", true).unwrap();

    let archives = tempfile::tempdir().unwrap();
    let backup = archives.path().join("schema-v21-backup");
    db.create_backup(&backup, false).unwrap();
    drop(db);
    let restored = tempfile::tempdir().unwrap();
    Trail::restore_backup(restored.path(), &backup, false).unwrap();

    let restored_db = Trail::open(restored.path()).unwrap();
    assert_eq!(restored_db.schema_user_version_for_test(), 21);
    let retirement = restored_db
        .lane_retirement(&spawned.lane_id)
        .unwrap()
        .expect("restored backup lost lane retirement provenance");
    assert_eq!(retirement.former_name, "retired-before-backup");
    assert_eq!(retirement.phase, LaneRetirementPhase::Completed);
}

#[test]
fn schema_newer_than_v21_is_refused_without_mutation() {
    let root = tempfile::tempdir().unwrap();
    Trail::init(root.path(), "main", InitImportMode::Empty, false).unwrap();
    let db_path = root.path().join(".trail/index/trail.sqlite");
    let conn = Connection::open(&db_path).unwrap();
    conn.pragma_update(None, "user_version", 22).unwrap();
    conn.execute(
        "UPDATE schema_meta SET value='22' WHERE key='schema.version'",
        [],
    )
    .unwrap();
    drop(conn);

    let error = match Trail::open(root.path()) {
        Ok(_) => panic!("schema newer than v21 unexpectedly opened"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("22") || error.to_string().contains("newer"),
        "{error}"
    );
    assert_eq!(
        Connection::open(db_path)
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        22
    );
}
