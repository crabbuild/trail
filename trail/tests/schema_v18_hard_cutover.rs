use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[cfg(target_os = "linux")]
use std::ffi::OsString;
#[cfg(target_os = "linux")]
use std::os::unix::ffi::OsStringExt;

use rusqlite::Connection;
use trail::{InitImportMode, Trail};

struct SchemaFixture {
    temp: tempfile::TempDir,
}

impl SchemaFixture {
    fn versioned(version: i64) -> Self {
        let fixture = Self::fresh_v18();
        let conn = Connection::open(fixture.sqlite_path()).unwrap();
        conn.pragma_update(None, "user_version", version).unwrap();
        conn.execute(
            "UPDATE schema_meta SET value = ?1 WHERE key = 'schema.version'",
            [version.to_string()],
        )
        .unwrap();
        drop(conn);
        fixture
    }

    fn partial_v18(missing_table: &str) -> Self {
        let fixture = Self::fresh_v18();
        let conn = Connection::open(fixture.sqlite_path()).unwrap();
        conn.execute_batch("PRAGMA foreign_keys = OFF;").unwrap();
        conn.execute_batch(&format!("DROP TABLE {missing_table};"))
            .unwrap();
        drop(conn);
        fixture
    }

    fn with_sql(mutator: impl FnOnce(&Connection)) -> Self {
        let fixture = Self::fresh_v18();
        let conn = Connection::open(fixture.sqlite_path()).unwrap();
        mutator(&conn);
        drop(conn);
        fixture
    }

    fn mutated_master_sql(object: &str, from: &str, to: &str) -> Self {
        Self::with_sql(|conn| {
            conn.execute_batch("PRAGMA writable_schema = ON;").unwrap();
            let changed = conn
                .execute(
                    "UPDATE sqlite_master SET sql = replace(sql, ?2, ?3) WHERE name = ?1",
                    (object, from, to),
                )
                .unwrap();
            assert_eq!(changed, 1);
            conn.execute_batch("PRAGMA writable_schema = OFF;").unwrap();
            let schema_version: i64 = conn
                .query_row("PRAGMA schema_version", [], |row| row.get(0))
                .unwrap();
            conn.pragma_update(None, "schema_version", schema_version + 1)
                .unwrap();
        })
    }

    fn fresh_v18() -> Self {
        let temp = tempfile::tempdir().unwrap();
        Trail::init(temp.path(), "main", InitImportMode::Empty, false).unwrap();
        Self { temp }
    }

    fn root(&self) -> &Path {
        self.temp.path()
    }

    fn sqlite_path(&self) -> PathBuf {
        self.root().join(".trail/index/trail.sqlite")
    }

    fn snapshot_tree_bytes(&self) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            let mut entries = fs::read_dir(path)
                .unwrap()
                .collect::<Result<Vec<_>, _>>()
                .unwrap();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                let file_type = entry.file_type().unwrap();
                if file_type.is_dir() {
                    visit(root, &path, files);
                } else if file_type.is_file() {
                    files.insert(
                        path.strip_prefix(root).unwrap().to_path_buf(),
                        fs::read(path).unwrap(),
                    );
                }
            }
        }

        let trail_dir = self.root().join(".trail");
        let mut files = BTreeMap::new();
        visit(&trail_dir, &trail_dir, &mut files);
        files
    }

    fn use_slatedb_backend(&self) {
        let config_path = self.root().join(".trail/config.toml");
        let config = fs::read_to_string(&config_path).unwrap();
        let config = config.replace(
            "prolly_backend = \"sqlite\"",
            "prolly_backend = \"slatedb\"",
        );
        assert!(config.contains("prolly_backend = \"slatedb\""));
        fs::write(config_path, config).unwrap();
    }
}

fn open_error(root: &Path) -> trail::Error {
    match Trail::open(root) {
        Ok(_) => panic!("existing incompatible schema was opened"),
        Err(err) => err,
    }
}

fn assert_tree_unchanged(fixture: &SchemaFixture, before: &BTreeMap<PathBuf, Vec<u8>>) {
    let after = fixture.snapshot_tree_bytes();
    let changes = before
        .keys()
        .chain(after.keys())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .filter_map(|path| {
            let old = before.get(path);
            let new = after.get(path);
            (old != new).then(|| {
                format!(
                    "{}: {:?} -> {:?} bytes",
                    path.display(),
                    old.map(Vec::len),
                    new.map(Vec::len)
                )
            })
        })
        .collect::<Vec<_>>();
    assert!(changes.is_empty(), "workspace bytes changed: {changes:?}");
}

#[test]
fn existing_v17_is_rejected_without_mutating_any_trail_byte() {
    let fixture = SchemaFixture::versioned(17);
    let before = fixture.snapshot_tree_bytes();
    let err = open_error(fixture.root());
    assert_eq!(err.code(), "SCHEMA_REINITIALIZE_REQUIRED");
    match &err {
        trail::Error::SchemaReinitializeRequired { found, guidance } => {
            assert_eq!(
                found,
                "database corrupt: found version 17; expected version 18"
            );
            assert_eq!(
                guidance,
                "back up this workspace, then run `trail init --force` to create schema v18"
            );
        }
        other => panic!("unexpected error: {other}"),
    }
    assert_eq!(
        err.to_string(),
        "workspace schema database corrupt: found version 17; expected version 18 cannot be opened; back up this workspace, then run `trail init --force` to create schema v18"
    );
    assert_tree_unchanged(&fixture, &before);
}

#[test]
fn missing_or_changed_prolly_schema_is_rejected_without_mutation() {
    let fixtures = [
        SchemaFixture::with_sql(|conn| {
            conn.execute_batch("DROP TABLE prolly_hints;").unwrap();
        }),
        SchemaFixture::mutated_master_sql("prolly_nodes", "node BLOB NOT NULL", "node BLOB"),
    ];

    for fixture in fixtures {
        let before = fixture.snapshot_tree_bytes();
        let err = open_error(fixture.root());
        assert_eq!(err.code(), "SCHEMA_REINITIALIZE_REQUIRED");
        assert_tree_unchanged(&fixture, &before);
    }
}

#[test]
fn extra_views_and_triggers_are_rejected_without_mutation() {
    let fixtures = [
        SchemaFixture::with_sql(|conn| {
            conn.execute_batch(
                "CREATE VIEW changed_paths AS SELECT normalized_path FROM changed_path_entries;",
            )
            .unwrap();
        }),
        SchemaFixture::with_sql(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER trail_extra_trigger AFTER INSERT ON schema_meta
                 BEGIN SELECT NEW.key; END;",
            )
            .unwrap();
        }),
    ];

    for fixture in fixtures {
        let before = fixture.snapshot_tree_bytes();
        let err = open_error(fixture.root());
        assert_eq!(err.code(), "SCHEMA_REINITIALIZE_REQUIRED");
        assert_tree_unchanged(&fixture, &before);
    }
}

#[test]
fn slatedb_backend_runs_sql_schema_preflight_before_mutable_backend_open() {
    let fixture = SchemaFixture::versioned(17);
    fixture.use_slatedb_backend();
    let err = open_error(fixture.root());
    assert!(matches!(
        err,
        trail::Error::SchemaReinitializeRequired { .. }
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn existing_schema_opens_from_a_non_utf8_workspace_path() {
    let temp = tempfile::tempdir().unwrap();
    let workspace = temp
        .path()
        .join(OsString::from_vec(b"workspace-\xff".to_vec()));
    fs::create_dir(&workspace).unwrap();
    Trail::init(&workspace, "main", InitImportMode::Empty, false).unwrap();

    Trail::open(&workspace).unwrap();
}

#[test]
fn partial_v18_is_rejected_without_repair() {
    let fixture = SchemaFixture::partial_v18("changed_path_scopes");
    let before = fixture.snapshot_tree_bytes();
    let err = open_error(fixture.root());
    assert_eq!(err.code(), "SCHEMA_REINITIALIZE_REQUIRED");
    assert_tree_unchanged(&fixture, &before);
}

#[test]
fn existing_v0_v17_and_v19_are_rejected_without_mutation() {
    for version in [0, 17, 19] {
        let fixture = SchemaFixture::versioned(version);
        let before = fixture.snapshot_tree_bytes();
        let err = open_error(fixture.root());
        assert_eq!(err.code(), "SCHEMA_REINITIALIZE_REQUIRED");
        assert!(err.to_string().contains("trail init --force"));
        assert_tree_unchanged(&fixture, &before);
    }
}

#[test]
fn fresh_init_creates_the_exact_v18_ledger_shape() {
    let fixture = SchemaFixture::fresh_v18();
    let conn = Connection::open(fixture.sqlite_path()).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        18
    );

    let metadata = [
        ("schema.version", "18"),
        ("changed_path.observer_log_format_min", "1"),
        ("changed_path.observer_log_format_max", "1"),
    ];
    for (key, expected) in metadata {
        let value: String = conn
            .query_row(
                "SELECT value FROM schema_meta WHERE key = ?1",
                [key],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(value, expected);
    }

    for table in [
        "changed_path_scopes",
        "changed_path_entries",
        "changed_path_prefixes",
        "changed_path_policy_dependencies",
        "changed_path_intents",
        "changed_path_intent_paths",
        "changed_path_intent_prefixes",
        "changed_path_reconciliations",
        "changed_path_observer_segments",
        "changed_path_observer_owners",
    ] {
        let sql: String = conn
            .query_row(
                "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = ?1",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert!(!sql.contains("IF NOT EXISTS"));
        assert!(!sql.contains("legacy_reconcile_required"));
    }
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_master WHERE name = 'changed_path_observer_leases'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );

    let entry_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'changed_path_entries'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(entry_sql.contains("normalized_path TEXT COLLATE BINARY NOT NULL"));

    let scope_columns = conn
        .prepare(
            "SELECT name, type, [notnull], dflt_value FROM pragma_table_xinfo(\
                'changed_path_scopes'\
             ) WHERE name LIKE 'max_%' ORDER BY cid",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, String>(3)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        scope_columns,
        vec![
            (
                "max_candidate_rows".into(),
                "INTEGER".into(),
                1,
                "250000".into()
            ),
            (
                "max_prefix_rows".into(),
                "INTEGER".into(),
                1,
                "16384".into()
            ),
            (
                "max_observer_log_bytes".into(),
                "INTEGER".into(),
                1,
                "268435456".into(),
            ),
            (
                "max_segment_bytes".into(),
                "INTEGER".into(),
                1,
                "16777216".into(),
            ),
            (
                "max_unfolded_tail_records".into(),
                "INTEGER".into(),
                1,
                "65536".into(),
            ),
        ]
    );
    let scope_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'changed_path_scopes'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    for check in [
        "CHECK(max_candidate_rows>0)",
        "CHECK(max_prefix_rows>0)",
        "CHECK(max_observer_log_bytes>0)",
        "CHECK(max_segment_bytes>0 AND max_segment_bytes<=max_observer_log_bytes)",
        "CHECK(max_unfolded_tail_records>0)",
    ] {
        assert!(scope_sql.contains(check), "missing {check}");
    }

    let policy_sql: String = conn
        .query_row(
            "SELECT sql FROM sqlite_master WHERE name = 'changed_path_policy_dependencies'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert!(policy_sql.contains("dependency_identity TEXT COLLATE BINARY NOT NULL"));
    assert!(policy_sql.contains("WITHOUT ROWID"));
    assert!(policy_sql.contains("content_identity BLOB NOT NULL"));
    assert!(policy_sql.contains("metadata_identity BLOB NOT NULL"));
    assert!(
        policy_sql.contains("'ignore'"),
        "the walker-native .ignore source requires a distinct durable dependency role"
    );

    let policy_index_columns = conn
        .prepare(
            "SELECT name, coll FROM pragma_index_xinfo(\
                'changed_path_policy_dependencies_generation_idx'\
             ) WHERE key = 1 ORDER BY seqno",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        policy_index_columns,
        vec![
            ("scope_id".to_string(), "BINARY".to_string()),
            ("generation".to_string(), "BINARY".to_string()),
            ("last_source_sequence".to_string(), "BINARY".to_string()),
        ]
    );

    let delete_action: String = conn
        .query_row(
            "SELECT on_delete FROM pragma_foreign_key_list('changed_path_entries') \
             WHERE [from] = 'intent_id'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(delete_action, "SET NULL");

    let index_columns = conn
        .prepare(
            "SELECT name, coll FROM pragma_index_xinfo('changed_path_entries_sequence_idx') \
             WHERE key = 1 ORDER BY seqno",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        index_columns,
        vec![
            ("scope_id".to_string(), "BINARY".to_string()),
            ("last_sequence".to_string(), "BINARY".to_string()),
        ]
    );
}

#[test]
fn malformed_v18_attributes_are_rejected_read_only() {
    let fixtures = [
        SchemaFixture::mutated_master_sql(
            "changed_path_scopes",
            "DEFAULT 'reconciling'",
            "DEFAULT 'trusted'",
        ),
        SchemaFixture::mutated_master_sql(
            "changed_path_scopes",
            "DEFAULT 250000 CHECK(max_candidate_rows>0)",
            "DEFAULT 250000 CHECK(max_candidate_rows>=0)",
        ),
        SchemaFixture::mutated_master_sql(
            "changed_path_entries",
            "normalized_path TEXT COLLATE BINARY",
            "normalized_path TEXT COLLATE NOCASE",
        ),
        SchemaFixture::mutated_master_sql(
            "changed_path_entries",
            "ON DELETE SET NULL",
            "ON DELETE CASCADE",
        ),
        SchemaFixture::mutated_master_sql(
            "changed_path_entries_sequence_idx",
            "scope_id, last_sequence",
            "last_sequence, scope_id",
        ),
        SchemaFixture::mutated_master_sql(
            "changed_path_observer_segments",
            "CHECK (log_format_version = 1)",
            "CHECK (log_format_version BETWEEN 1 AND 2)",
        ),
        SchemaFixture::with_sql(|conn| {
            conn.execute(
                "UPDATE schema_meta SET value = '2' \
                 WHERE key = 'changed_path.observer_log_format_max'",
                [],
            )
            .unwrap();
        }),
    ];

    for fixture in fixtures {
        let before = fixture.snapshot_tree_bytes();
        let err = open_error(fixture.root());
        assert_eq!(err.code(), "SCHEMA_REINITIALIZE_REQUIRED");
        assert_tree_unchanged(&fixture, &before);
    }
}
