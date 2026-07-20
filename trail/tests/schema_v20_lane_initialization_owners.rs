use std::fs;
use std::path::PathBuf;

use rusqlite::{params, Connection};
use trail::test_support::SchemaV20MigrationBoundary;
use trail::Trail;

const LANE_INITIALIZATIONS_V19: &str = r#"
CREATE TABLE lane_initializations (
    initialization_id TEXT PRIMARY KEY,
    lane_name TEXT NOT NULL UNIQUE,
    lane_id TEXT NOT NULL,
    request_fingerprint TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (phase IN
        ('reserved','materialized','associated','observer_ready','repair_required')),
    workdir TEXT,
    materialization_json TEXT,
    last_error_code TEXT,
    last_error_message TEXT,
    repair_command TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX lane_initializations_phase_updated_idx
    ON lane_initializations(phase, updated_at);
"#;

const EXPECTED_OWNER_COLUMNS: &[(&str, &str, bool, bool)] = &[
    ("initialization_id", "TEXT", false, true),
    ("owner_token", "TEXT", true, false),
    ("owner_generation", "INTEGER", true, false),
    ("owner_pid", "INTEGER", true, false),
    ("owner_process_start_identity", "TEXT", true, false),
    ("acquired_at", "INTEGER", true, false),
    ("heartbeat_at", "INTEGER", true, false),
];

struct SchemaV20Fixture {
    root: tempfile::TempDir,
    db_path: PathBuf,
}

impl SchemaV20Fixture {
    fn from_v19() -> Self {
        let fixture = Self::from_v18();
        let conn = Connection::open(&fixture.db_path).unwrap();
        conn.execute_batch(LANE_INITIALIZATIONS_V19).unwrap();
        conn.execute(
            "UPDATE schema_meta SET value='19',updated_at=1 WHERE key='schema.version'",
            [],
        )
        .unwrap();
        conn.execute(
            "UPDATE schema_meta SET value=?1,updated_at=1 WHERE key='app.version'",
            [env!("CARGO_PKG_VERSION")],
        )
        .unwrap();
        conn.pragma_update(None, "user_version", 19).unwrap();
        drop(conn);
        fixture
    }

    fn from_v18_with_lane() -> Self {
        let fixture = Self::from_v18();
        let workdir = fixture.root.path().join(".trail/worktrees/legacy");
        fs::create_dir_all(workdir.join(".trail")).unwrap();
        let conn = Connection::open(&fixture.db_path).unwrap();
        let (change_id, root_id, operation_id, generation): (String, String, String, i64) = conn
            .query_row(
                "SELECT change_id,root_id,operation_id,generation
                 FROM refs WHERE name='refs/branches/main'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        conn.execute(
            "INSERT INTO refs(name,change_id,root_id,operation_id,generation,updated_at)
             VALUES('refs/lanes/legacy',?1,?2,?3,?4,1)",
            params![change_id, root_id, operation_id, generation],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO lanes(lane_id,name,kind,created_at)
             VALUES('lane_legacy','legacy','coding-lane',1)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO lane_branches(
                 lane_id,ref_name,base_change,head_change,base_root,head_root,
                 workdir,status,created_at,updated_at)
             VALUES('lane_legacy','refs/lanes/legacy',?1,?1,?2,?2,?3,'active',1,1)",
            params![change_id, root_id, workdir.to_string_lossy().as_ref()],
        )
        .unwrap();
        drop(conn);
        fixture
    }

    fn from_v18() -> Self {
        let root = tempfile::tempdir().unwrap();
        trail::test_support::create_schema_v18_fixture(root.path()).unwrap();
        let db_path = root.path().join(".trail/index/trail.sqlite");
        Self { root, db_path }
    }

    fn open(&self) -> Trail {
        self.open_result().unwrap()
    }

    fn open_result(&self) -> trail::Result<Trail> {
        Trail::open(self.root.path())
    }

    fn install_failure(&self, boundary: SchemaV20MigrationBoundary) {
        trail::test_support::install_schema_v20_migration_failure(&self.db_path, boundary);
    }

    fn raw_user_version(&self) -> i64 {
        Connection::open(&self.db_path)
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .unwrap()
    }

    fn table_exists(&self, name: &str) -> bool {
        Connection::open(&self.db_path)
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [name],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn owner_columns(&self) -> Vec<(String, String, bool, bool)> {
        Connection::open(&self.db_path)
            .unwrap()
            .prepare("SELECT name,type,[notnull],pk FROM pragma_table_info('lane_initialization_owners') ORDER BY cid")
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

    fn owner_count(&self) -> i64 {
        self.count("lane_initialization_owners")
    }

    fn initialization_count(&self) -> i64 {
        self.count("lane_initializations")
    }

    fn count(&self, table: &str) -> i64 {
        let conn = Connection::open(&self.db_path).unwrap();
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }
}

#[test]
fn v19_open_migrates_owner_authority_atomically_to_v20() {
    let fixture = SchemaV20Fixture::from_v19();
    let db = fixture.open();
    assert_eq!(db.schema_user_version_for_test(), 20);
    let owner_columns = fixture.owner_columns();
    let owner_columns = owner_columns
        .iter()
        .map(|(name, kind, not_null, primary_key)| {
            (name.as_str(), kind.as_str(), *not_null, *primary_key)
        })
        .collect::<Vec<_>>();
    assert_eq!(owner_columns, EXPECTED_OWNER_COLUMNS);
    assert_eq!(fixture.owner_count(), 0);
}

#[test]
fn v18_open_runs_v19_backfill_then_v20_owner_migration() {
    let fixture = SchemaV20Fixture::from_v18_with_lane();
    let db = fixture.open();
    assert_eq!(db.schema_user_version_for_test(), 20);
    assert_eq!(fixture.initialization_count(), 1);
    assert_eq!(fixture.owner_count(), 0);
}

#[test]
fn v20_migration_fault_rolls_back_table_metadata_and_user_version() {
    let fixture = SchemaV20Fixture::from_v19();
    fixture.install_failure(SchemaV20MigrationBoundary::AfterDdlBeforeUserVersion);
    assert!(fixture.open_result().is_err());
    assert_eq!(fixture.raw_user_version(), 19);
    assert!(!fixture.table_exists("lane_initialization_owners"));
}
