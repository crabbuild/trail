use rusqlite::{params, Connection};
use trail::{InitImportMode, Trail};

#[test]
fn reconciliation_staging_has_separate_binary_guard_namespace() {
    let workspace = tempfile::tempdir().unwrap();
    Trail::init(workspace.path(), "main", InitImportMode::Empty, false).unwrap();
    let conn = Connection::open(workspace.path().join(".trail/index/trail.sqlite")).unwrap();

    let columns = conn
        .prepare("PRAGMA table_info(changed_path_reconciliation_guards)")
        .unwrap()
        .query_map([], |row| {
            Ok((row.get::<_, String>(1)?, row.get::<_, String>(2)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    assert!(columns.contains(&("relative_path".into(), "BLOB".into())));
    assert!(columns.contains(&("directory_identity".into(), "BLOB".into())));

    conn.pragma_update(None, "foreign_keys", false).unwrap();
    let arbitrary_path = b"\xff\0#directory-guard/61";
    conn.execute(
        "INSERT INTO changed_path_reconciliation_guards(
             attempt_id,relative_path,directory_identity,staged_at
         ) VALUES('independent-harness',?1,?2,0)",
        params![arbitrary_path.as_slice(), b"identity".as_slice()],
    )
    .unwrap();
    let stored: Vec<u8> = conn
        .query_row(
            "SELECT relative_path FROM changed_path_reconciliation_guards
             WHERE attempt_id='independent-harness'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(stored, arbitrary_path);
}

#[test]
fn compiled_harness_exercises_reconciliation_oracle() {
    trail::test_support::changed_path_reconciliation_oracle().unwrap();
}

#[test]
fn compiled_harness_exercises_fail_closed_publication_races() {
    trail::test_support::changed_path_reconciliation_races().unwrap();
}

#[test]
fn compiled_harness_exercises_callback_spooling() {
    trail::test_support::changed_path_reconciliation_callback_spool().unwrap();
}
