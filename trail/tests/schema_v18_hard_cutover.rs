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
    wal_writer: Option<Connection>,
}

impl SchemaFixture {
    fn predecessor_v18() -> Self {
        let temp = tempfile::tempdir().unwrap();
        trail::test_support::create_schema_v18_fixture(temp.path()).unwrap();
        Self {
            temp,
            wal_writer: None,
        }
    }

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

    fn with_persistent_wal(mutator: impl FnOnce(&Connection)) -> Self {
        let mut fixture = Self::fresh_v18();
        let conn = Connection::open(fixture.sqlite_path()).unwrap();
        assert_eq!(
            conn.pragma_update_and_check(None, "journal_mode", "WAL", |row| {
                row.get::<_, String>(0)
            })
            .unwrap()
            .to_ascii_lowercase(),
            "wal"
        );
        conn.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        mutator(&conn);
        assert!(fixture.sqlite_path().with_extension("sqlite-wal").exists());
        fixture.wal_writer = Some(conn);
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
        Self {
            temp,
            wal_writer: None,
        }
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

fn assert_persistent_wal_rejected_byte_invariantly(fixture: &SchemaFixture) {
    let before = fixture.snapshot_tree_bytes();
    assert!(
        before.keys().any(|path| path.ends_with("trail.sqlite-wal")),
        "fixture did not retain a WAL generation"
    );
    let err = open_error(fixture.root());
    assert_eq!(err.code(), "SCHEMA_REINITIALIZE_REQUIRED");
    assert_tree_unchanged(fixture, &before);
}

fn insert_malformed_retirement_graph(conn: &Connection, kind: &str) {
    conn.execute_batch(
        "INSERT INTO changed_path_scopes(
             scope_id,scope_kind,owner_id,scope_root,scope_root_identity,
             filesystem_identity,filesystem_kind,case_sensitive,ref_name,ref_generation,
             change_id,baseline_root_id,policy_fingerprint,policy_dependency_generation,
             provider_id,provider_identity,
             trust_state,trust_reason,retired_at,created_at,updated_at)
         VALUES('scope-a','workspace','owner-a','','root-id','fs-id','native',1,
                'refs/heads/main',1,'change-a','root-a','policy-a',1,
                'provider','',
                'untrusted_gap','scope_retired',1,1,1);
         INSERT INTO changed_path_observer_owners(
             scope_id,epoch,owner_token,provider_id,provider_identity,lease_state,
             fence_nonce,acquired_at,heartbeat_at,expires_at,updated_at)
         VALUES('scope-a',1,
                'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                'provider','','revoked',
                X'0000000000000000000000000000000000000000000000000000000000000000',
                1,1,1,1);
         INSERT INTO changed_path_observer_segments(
             scope_id,epoch,segment_id,owner_token,provider_id,first_sequence,
             durable_end_offset,folded_end_offset,segment_path,state,retirement_source_state,
             retirement_file_length,retirement_file_hash,retirement_durable_hash,
             retirement_source_device,retirement_source_inode,created_at,updated_at)
         VALUES
             ('scope-a',1,'segment-a',
              'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
              'provider',1,0,0,'segment-a.cpl','retired','open',0,
              '0000000000000000000000000000000000000000000000000000000000000000',
              '0000000000000000000000000000000000000000000000000000000000000000',
              '7','8',1,1),
             ('scope-a',1,'segment-b',
              'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
              'provider',2,0,0,'segment-b.cpl','retired','open',0,
              '0000000000000000000000000000000000000000000000000000000000000000',
              '0000000000000000000000000000000000000000000000000000000000000000',
              '9','10',1,1);",
    )
    .unwrap();
    let allocation_state = if kind == "state_invalid" {
        "allocated"
    } else {
        "bound"
    };
    let bound_at = if allocation_state == "bound" {
        "1"
    } else {
        "NULL"
    };
    conn.execute_batch(&format!(
        "INSERT INTO changed_path_segment_quarantine_allocations(
             attempt_nonce,scope_id,epoch,segment_id,quarantine_leaf,
             scope_directory_device,scope_directory_inode,identity_policy,
             source_segment_device,source_segment_inode,quarantine_device,quarantine_inode,state,
             created_at,updated_at,allocated_at,bound_at)
         VALUES(
             'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
             'scope-a',1,'segment-a','.trail-delete-a.cplq','1','2',
             'direct_noreplace_same_directory_v1','7','8','7','8','{allocation_state}',1,1,1,{bound_at});"
    ))
    .unwrap();
    if kind == "cross_wired" {
        conn.execute_batch(
            "INSERT INTO changed_path_segment_quarantine_allocations(
                 attempt_nonce,scope_id,epoch,segment_id,quarantine_leaf,
                 scope_directory_device,scope_directory_inode,identity_policy,
                 source_segment_device,source_segment_inode,quarantine_device,quarantine_inode,state,
                 created_at,updated_at,allocated_at,bound_at)
             VALUES(
                 'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb',
                 'scope-a',1,'segment-b','.trail-delete-b.cplq','1','2',
                 'direct_noreplace_same_directory_v1','9','10','9','10','bound',1,1,1,1);",
        )
        .unwrap();
    }
    if kind != "orphan" {
        let allocation_nonce = if kind == "cross_wired" {
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb"
        } else {
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
        };
        conn.execute_batch(&format!(
            "INSERT INTO changed_path_segment_deletions(
                 scope_id,epoch,segment_id,original_leaf,quarantine_leaf,allocation_nonce,
                 log_format_version,provider_id,folded_end_offset,
                 retirement_continuity_generation,retirement_fence_nonce,
                 scope_directory_device,scope_directory_inode,quarantine_device,quarantine_inode,
                 segment_device,segment_inode,file_length,file_hash,durable_end_offset,
                 durable_hash,max_observer_log_bytes,max_segment_bytes,max_unfolded_tail_records,
                 owner_token,first_sequence,last_sequence,previous_segment_id,
                 previous_segment_hash,source_state,state,created_at,updated_at,completed_at)
             VALUES('scope-a',1,'segment-a','segment-a.cpl','.trail-delete-a.cplq',
                    '{allocation_nonce}',1,'provider',0,1,
                    X'0000000000000000000000000000000000000000000000000000000000000000',
                    '1','2','7','8','7','8',0,
                    '0000000000000000000000000000000000000000000000000000000000000000',0,
                    '0000000000000000000000000000000000000000000000000000000000000000',
                    1024,512,16,
                    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
                    1,NULL,NULL,
                    '0000000000000000000000000000000000000000000000000000000000000000',
                    'open','quiesced',1,1,1);"
        ))
        .unwrap();
    }
}

fn insert_source_quarantine_cross_wire(conn: &Connection) {
    insert_malformed_retirement_graph(conn, "valid");
    conn.execute(
        "DELETE FROM changed_path_observer_segments
         WHERE scope_id='scope-a' AND segment_id='segment-b'",
        [],
    )
    .unwrap();
    conn.execute_batch(
        "PRAGMA ignore_check_constraints=ON;
         UPDATE changed_path_segment_quarantine_allocations
         SET quarantine_device='17',quarantine_inode='18'
         WHERE scope_id='scope-a' AND segment_id='segment-a';
         UPDATE changed_path_segment_deletions
         SET quarantine_device='17',quarantine_inode='18'
         WHERE scope_id='scope-a' AND segment_id='segment-a';
         PRAGMA ignore_check_constraints=OFF;",
    )
    .unwrap();
}

fn insert_policy_scope(conn: &Connection) {
    conn.execute_batch(
        "INSERT INTO changed_path_scopes(
             scope_id,scope_kind,owner_id,scope_root,scope_root_identity,
             filesystem_identity,filesystem_kind,case_sensitive,ref_name,ref_generation,
             change_id,baseline_root_id,policy_fingerprint,policy_dependency_generation,
             trust_state,trust_reason,created_at,updated_at)
         VALUES('policy-scope','workspace','policy-owner','','root-id','fs-id','native',1,
                'refs/heads/main',1,'change-a','root-a','policy-a',1,
                'stale_baseline','policy_fixture',1,1);",
    )
    .unwrap();
}

fn insert_policy_dependency(
    conn: &Connection,
    identity: &str,
    kind: &str,
    content_identity: &[u8],
    generation: i64,
) {
    conn.execute_batch("PRAGMA ignore_check_constraints=ON;")
        .unwrap();
    conn.execute(
        "INSERT INTO changed_path_policy_dependencies(
             scope_id,dependency_identity,dependency_kind,content_identity,
             metadata_identity,observable,generation,last_source_sequence,created_at,updated_at)
         VALUES('policy-scope',?1,?2,?3,X'73796e7468657469632d7631',1,?4,0,1,1)",
        (identity, kind, content_identity, generation),
    )
    .unwrap();
    conn.execute_batch("PRAGMA ignore_check_constraints=OFF;")
        .unwrap();
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
                "database corrupt: found version 17; expected version 19"
            );
            assert_eq!(
                guidance,
                "back up this workspace, then run `trail init --force` to create schema v19"
            );
        }
        other => panic!("unexpected error: {other}"),
    }
    assert_eq!(
        err.to_string(),
        "workspace schema database corrupt: found version 17; expected version 19 cannot be opened; back up this workspace, then run `trail init --force` to create schema v19"
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
fn schema_v18_is_the_only_migratable_predecessor() {
    let predecessor = SchemaFixture::predecessor_v18();
    Trail::open(predecessor.root()).unwrap();
    assert_eq!(
        Connection::open(predecessor.sqlite_path())
            .unwrap()
            .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        19
    );

    for version in [0, 17, 20] {
        let fixture = SchemaFixture::versioned(version);
        let before = fixture.snapshot_tree_bytes();
        let err = open_error(fixture.root());
        assert_eq!(err.code(), "SCHEMA_REINITIALIZE_REQUIRED");
        assert!(err.to_string().contains("trail init --force"));
        assert_tree_unchanged(&fixture, &before);
    }
}

#[test]
fn fresh_init_creates_the_exact_v19_ledger_shape() {
    let fixture = SchemaFixture::fresh_v18();
    let conn = Connection::open(fixture.sqlite_path()).unwrap();
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        19
    );

    let metadata = [
        ("schema.version", "19"),
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
        "changed_path_segment_quarantine_allocations",
        "changed_path_segment_deletions",
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

    let continuity_column: (String, String, i64, String) = conn
        .query_row(
            "SELECT name,type,\"notnull\",dflt_value
             FROM pragma_table_info('changed_path_scopes')
             WHERE name='continuity_generation'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .unwrap();
    assert_eq!(
        continuity_column,
        (
            "continuity_generation".into(),
            "INTEGER".into(),
            1,
            "1".into()
        )
    );

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

    let allocation_columns = conn
        .prepare(
            "SELECT name FROM pragma_table_info('changed_path_segment_quarantine_allocations')
             ORDER BY cid",
        )
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert_eq!(
        allocation_columns,
        vec![
            "attempt_nonce",
            "scope_id",
            "epoch",
            "segment_id",
            "quarantine_leaf",
            "scope_directory_device",
            "scope_directory_inode",
            "identity_policy",
            "source_segment_device",
            "source_segment_inode",
            "quarantine_device",
            "quarantine_inode",
            "observed_conflict_device",
            "observed_conflict_inode",
            "retained_reason",
            "state",
            "created_at",
            "updated_at",
            "allocated_at",
            "bound_at",
            "abandoned_at",
        ]
    );
    let deletion_allocation_column: (String, String, i64) = conn
        .query_row(
            "SELECT name,type,[notnull]
             FROM pragma_table_info('changed_path_segment_deletions')
             WHERE name='allocation_nonce'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(
        deletion_allocation_column,
        ("allocation_nonce".into(), "TEXT".into(), 1)
    );

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

#[test]
fn malformed_retirement_graph_is_rejected_read_only() {
    for kind in ["orphan", "cross_wired", "state_invalid"] {
        let fixture = SchemaFixture::with_sql(|conn| {
            insert_malformed_retirement_graph(conn, kind);
        });
        let before = fixture.snapshot_tree_bytes();
        let err = open_error(fixture.root());
        assert_eq!(err.code(), "SCHEMA_REINITIALIZE_REQUIRED", "kind={kind}");
        assert_tree_unchanged(&fixture, &before);
    }
}

#[test]
fn source_segment_and_quarantine_identity_cross_wire_is_rejected_read_only() {
    let fixture = SchemaFixture::with_sql(insert_source_quarantine_cross_wire);
    let before = fixture.snapshot_tree_bytes();
    let err = open_error(fixture.root());
    assert_eq!(err.code(), "SCHEMA_REINITIALIZE_REQUIRED");
    assert_tree_unchanged(&fixture, &before);
}

#[test]
fn malformed_policy_dependency_rows_are_rejected_read_only() {
    let content_identity = [7_u8; 32];
    let fixtures = [
        SchemaFixture::with_sql(|conn| {
            insert_policy_scope(conn);
            insert_policy_dependency(conn, "builtin:recording-policy", "builtin", &[7], 1);
        }),
        SchemaFixture::with_sql(|conn| {
            insert_policy_scope(conn);
            insert_policy_dependency(
                conn,
                "builtin:recording-policy",
                "builtin",
                &content_identity,
                2,
            );
        }),
        SchemaFixture::with_sql(|conn| {
            insert_policy_scope(conn);
            insert_policy_dependency(
                conn,
                "synthetic:not-a-path",
                "gitignore",
                &content_identity,
                1,
            );
        }),
        SchemaFixture::with_sql(|conn| {
            insert_policy_scope(conn);
            insert_policy_dependency(
                conn,
                "path:2f746d702f612f2e2e2f62",
                "gitignore",
                &content_identity,
                1,
            );
        }),
        SchemaFixture::with_sql(|conn| {
            insert_policy_scope(conn);
            insert_policy_dependency(
                conn,
                "path:2f746d702f706f6c696379",
                "builtin",
                &content_identity,
                1,
            );
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
fn malformed_scope_segment_allocation_deletion_graph_is_rejected_read_only() {
    let fixtures = [
        SchemaFixture::with_sql(|conn| {
            insert_malformed_retirement_graph(conn, "orphan");
        }),
        SchemaFixture::with_sql(|conn| {
            insert_malformed_retirement_graph(conn, "state_invalid");
            conn.execute_batch(
                "UPDATE changed_path_scopes
                 SET retired_at=NULL,trust_reason='active_scope'
                 WHERE scope_id='scope-a';
                 UPDATE changed_path_observer_segments
                 SET state='open' WHERE scope_id='scope-a';",
            )
            .unwrap();
        }),
        SchemaFixture::with_sql(|conn| {
            insert_malformed_retirement_graph(conn, "valid");
            conn.execute(
                "DELETE FROM changed_path_observer_segments
                 WHERE scope_id='scope-a' AND segment_id='segment-b'",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE changed_path_segment_deletions
                 SET original_leaf='different.cpl' WHERE scope_id='scope-a'",
                [],
            )
            .unwrap();
        }),
        SchemaFixture::with_sql(|conn| {
            insert_malformed_retirement_graph(conn, "valid");
            conn.execute(
                "DELETE FROM changed_path_observer_segments
                 WHERE scope_id='scope-a' AND segment_id='segment-b'",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE changed_path_segment_deletions
                 SET owner_token='cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc'
                 WHERE scope_id='scope-a'",
                [],
            )
            .unwrap();
        }),
        SchemaFixture::with_sql(|conn| {
            insert_malformed_retirement_graph(conn, "cross_wired");
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
fn persistent_wal_malformed_retirement_graph_is_rejected_byte_invariantly() {
    let fixture = SchemaFixture::with_persistent_wal(|conn| {
        insert_malformed_retirement_graph(conn, "state_invalid");
    });
    assert_persistent_wal_rejected_byte_invariantly(&fixture);
}

#[test]
fn persistent_wal_cross_wired_retirement_identity_is_rejected_byte_invariantly() {
    let fixture = SchemaFixture::with_persistent_wal(insert_source_quarantine_cross_wire);
    assert_persistent_wal_rejected_byte_invariantly(&fixture);
}

#[test]
fn persistent_wal_malformed_policy_dependencies_are_rejected_byte_invariantly() {
    let malformed_generation = SchemaFixture::with_persistent_wal(|conn| {
        insert_policy_scope(conn);
        insert_policy_dependency(conn, "builtin:recording-policy", "builtin", &[7_u8; 32], 2);
    });
    assert_persistent_wal_rejected_byte_invariantly(&malformed_generation);

    let malformed_path = SchemaFixture::with_persistent_wal(|conn| {
        insert_policy_scope(conn);
        insert_policy_dependency(
            conn,
            "path:2f746d702f612f2e2e2f62",
            "gitignore",
            &[7_u8; 32],
            1,
        );
    });
    assert_persistent_wal_rejected_byte_invariantly(&malformed_path);
}

#[test]
fn persistent_wal_schema_fk_and_partial_generations_are_rejected_byte_invariantly() {
    let malformed_schema = SchemaFixture::with_persistent_wal(|conn| {
        conn.execute_batch("PRAGMA writable_schema=ON;").unwrap();
        conn.execute(
            "UPDATE sqlite_master SET sql=replace(sql,'CHECK (state = ''quiesced'')',
                    'CHECK (state IN (''quiesced'',''deleted''))')
             WHERE name='changed_path_segment_deletions'",
            [],
        )
        .unwrap();
        conn.execute_batch("PRAGMA writable_schema=OFF;").unwrap();
        let version: i64 = conn
            .query_row("PRAGMA schema_version", [], |row| row.get(0))
            .unwrap();
        conn.pragma_update(None, "schema_version", version + 1)
            .unwrap();
    });
    assert_persistent_wal_rejected_byte_invariantly(&malformed_schema);

    let orphan_policy = SchemaFixture::with_persistent_wal(|conn| {
        conn.pragma_update(None, "foreign_keys", false).unwrap();
        conn.execute(
            "INSERT INTO changed_path_policy_dependencies(
                 scope_id,dependency_identity,dependency_kind,content_identity,
                 metadata_identity,observable,generation,last_source_sequence,created_at,updated_at)
             VALUES('missing-scope','builtin:recording-policy','builtin',zeroblob(32),X'02',1,1,0,1,1)",
            [],
        )
        .unwrap();
    });
    assert_persistent_wal_rejected_byte_invariantly(&orphan_policy);

    let partial = SchemaFixture::with_persistent_wal(|conn| {
        conn.execute_batch("PRAGMA foreign_keys=OFF; DROP TABLE changed_path_policy_dependencies;")
            .unwrap();
    });
    assert_persistent_wal_rejected_byte_invariantly(&partial);
}

#[test]
fn valid_persistent_wal_generation_survives_snapshot_preflight_and_mutable_handoff() {
    let fixture = SchemaFixture::with_persistent_wal(|conn| {
        conn.execute(
            "UPDATE schema_meta SET value='wal-visible' WHERE key='app.version'",
            [],
        )
        .unwrap();
    });
    let opened = Trail::open(fixture.root()).unwrap();
    drop(opened);
    let conn = Connection::open(fixture.sqlite_path()).unwrap();
    let value: String = conn
        .query_row(
            "SELECT value FROM schema_meta WHERE key='app.version'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(value, "wal-visible");
}
