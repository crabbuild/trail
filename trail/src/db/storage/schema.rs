use super::*;
#[cfg(any(test, debug_assertions))]
use std::sync::{Mutex, OnceLock};

mod agent_capture;
mod changed_path_ledger;
mod ddl;
mod version;

type ProllySqliteIndexStructure = (String, Vec<String>);
type ProllySqliteTableStructure = (String, Vec<String>, Vec<ProllySqliteIndexStructure>);

const PROLLY_SQLITE_SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS prolly_nodes (
    cid  BLOB PRIMARY KEY NOT NULL,
    node BLOB NOT NULL
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS prolly_hints (
    namespace BLOB NOT NULL,
    key       BLOB NOT NULL,
    value     BLOB NOT NULL,
    PRIMARY KEY (namespace, key)
) WITHOUT ROWID;
CREATE TABLE IF NOT EXISTS prolly_roots (
    name     BLOB PRIMARY KEY NOT NULL,
    manifest BLOB NOT NULL
) WITHOUT ROWID;";

impl Trail {
    pub(crate) fn validate_schema_v20(conn: &Connection) -> Result<()> {
        validate_schema_v20(conn)
    }

    #[cfg(debug_assertions)]
    #[doc(hidden)]
    pub fn schema_user_version_for_test(&self) -> i64 {
        self.conn
            .query_row("PRAGMA user_version", [], |row| row.get(0))
            .expect("Trail connection must expose its schema user_version")
    }
}

pub(crate) fn validate_schema_v18_for_migration(conn: &Connection) -> Result<()> {
    validate_schema_version(conn, SCHEMA_V18_VERSION, false, false)
}

pub(crate) fn validate_schema_v19_for_migration(conn: &Connection) -> Result<()> {
    validate_schema_version(conn, SCHEMA_V19_VERSION, true, false)
}

pub(crate) fn validate_schema_v20(conn: &Connection) -> Result<()> {
    validate_schema_version(conn, TRAIL_SCHEMA_VERSION, true, true)
}

fn validate_schema_version(
    conn: &Connection,
    expected_version: i64,
    require_lane_initializations: bool,
    require_lane_initialization_owners: bool,
) -> Result<()> {
    let user_version = conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    if user_version != expected_version {
        return Err(Error::Corrupt(format!(
            "found version {user_version}; expected version {expected_version}"
        )));
    }
    if !ddl::base_schema_complete_for_version(conn, expected_version)? {
        return Err(Error::Corrupt("base schema v18 shape is incomplete".into()));
    }
    if !agent_capture::agent_capture_schema_complete(conn)? {
        return Err(Error::Corrupt(
            "agent capture schema v18 shape is incomplete".into(),
        ));
    }
    if !changed_path_ledger::changed_path_ledger_schema_complete(conn, expected_version)? {
        return Err(Error::Corrupt(
            "changed-path ledger schema v18 shape is incomplete".into(),
        ));
    }
    if require_lane_initializations {
        ddl::validate_lane_initializations_v19_shape(conn)?;
    } else if !ddl::lane_initialization_objects_absent(conn)? {
        return Err(Error::Corrupt(
            "schema v18 contains schema v19 lane initialization objects".into(),
        ));
    }
    if require_lane_initialization_owners {
        ddl::validate_lane_initialization_owners_v20_shape(conn)?;
    } else if !ddl::lane_initialization_owner_objects_absent(conn)? {
        return Err(Error::Corrupt(
            "pre-v20 schema contains lane initialization owner objects".into(),
        ));
    }
    Ok(())
}

pub(crate) fn migrate_schema_v18_to_v19(conn: &mut Connection) -> Result<()> {
    validate_schema_v18_for_migration(conn)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute_batch(ddl::LANE_INITIALIZATIONS_V19)?;
    fail_schema_v19_migration_if_installed(&tx)?;
    crate::db::lane::backfill_lane_initializations_v19(&tx)?;
    update_schema_version_metadata(&tx, SCHEMA_V19_VERSION)?;
    tx.pragma_update(None, "user_version", SCHEMA_V19_VERSION)?;
    validate_schema_v19_for_migration(&tx)?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn migrate_schema_v19_to_v20(conn: &mut Connection) -> Result<()> {
    validate_schema_v19_for_migration(conn)?;
    let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    tx.execute_batch(ddl::LANE_INITIALIZATION_OWNERS_V20)?;
    fail_schema_v20_migration_if_installed(&tx)?;
    update_schema_version_metadata(&tx, TRAIL_SCHEMA_VERSION)?;
    tx.pragma_update(None, "user_version", TRAIL_SCHEMA_VERSION)?;
    validate_schema_v20(&tx)?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn update_schema_version_metadata(
    tx: &rusqlite::Transaction<'_>,
    version: i64,
) -> Result<()> {
    let now = now_ts();
    tx.execute(
        "UPDATE schema_meta SET value=?1,updated_at=?2 WHERE key=?3",
        params![version.to_string(), now, SCHEMA_META_VERSION_KEY],
    )?;
    tx.execute(
        "UPDATE schema_meta SET value=?1,updated_at=?2 WHERE key=?3",
        params![env!("CARGO_PKG_VERSION"), now, SCHEMA_META_APP_VERSION_KEY],
    )?;
    Ok(())
}

#[cfg(any(test, debug_assertions))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SchemaV19MigrationBoundary {
    AfterDdlBeforeUserVersion,
}

#[cfg(any(test, debug_assertions))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SchemaV20MigrationBoundary {
    AfterDdlBeforeUserVersion,
}

#[cfg(any(test, debug_assertions))]
static SCHEMA_V19_MIGRATION_FAILURES: OnceLock<Mutex<std::collections::HashSet<PathBuf>>> =
    OnceLock::new();

#[cfg(any(test, debug_assertions))]
static SCHEMA_V20_MIGRATION_FAILURES: OnceLock<Mutex<std::collections::HashSet<PathBuf>>> =
    OnceLock::new();

#[cfg(any(test, debug_assertions))]
pub(crate) fn install_schema_v19_migration_failure(
    db_path: &Path,
    _boundary: SchemaV19MigrationBoundary,
) {
    SCHEMA_V19_MIGRATION_FAILURES
        .get_or_init(|| Mutex::new(std::collections::HashSet::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(canonicalize_lossless(db_path).unwrap_or_else(|_| db_path.to_path_buf()));
}

#[cfg(any(test, debug_assertions))]
pub(crate) fn clear_schema_v19_migration_failure(db_path: &Path) {
    if let Some(failures) = SCHEMA_V19_MIGRATION_FAILURES.get() {
        failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&canonicalize_lossless(db_path).unwrap_or_else(|_| db_path.to_path_buf()));
    }
}

#[cfg(any(test, debug_assertions))]
pub(crate) fn install_schema_v20_migration_failure(
    db_path: &Path,
    _boundary: SchemaV20MigrationBoundary,
) {
    SCHEMA_V20_MIGRATION_FAILURES
        .get_or_init(|| Mutex::new(std::collections::HashSet::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .insert(canonicalize_lossless(db_path).unwrap_or_else(|_| db_path.to_path_buf()));
}

#[cfg(any(test, debug_assertions))]
pub(crate) fn clear_schema_v20_migration_failure(db_path: &Path) {
    if let Some(failures) = SCHEMA_V20_MIGRATION_FAILURES.get() {
        failures
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&canonicalize_lossless(db_path).unwrap_or_else(|_| db_path.to_path_buf()));
    }
}

fn fail_schema_v19_migration_if_installed(conn: &Connection) -> Result<()> {
    #[cfg(any(test, debug_assertions))]
    {
        let path: String = conn.query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |row| row.get(0),
        )?;
        let path = PathBuf::from(path);
        let path = canonicalize_lossless(&path).unwrap_or(path);
        if SCHEMA_V19_MIGRATION_FAILURES
            .get_or_init(|| Mutex::new(std::collections::HashSet::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&path)
        {
            return Err(Error::Corrupt(
                "injected schema v19 migration failure after DDL".into(),
            ));
        }
    }
    #[cfg(not(any(test, debug_assertions)))]
    let _ = conn;
    Ok(())
}

fn fail_schema_v20_migration_if_installed(conn: &Connection) -> Result<()> {
    #[cfg(any(test, debug_assertions))]
    {
        let path: String = conn.query_row(
            "SELECT file FROM pragma_database_list WHERE name='main'",
            [],
            |row| row.get(0),
        )?;
        let path = PathBuf::from(path);
        let path = canonicalize_lossless(&path).unwrap_or(path);
        if SCHEMA_V20_MIGRATION_FAILURES
            .get_or_init(|| Mutex::new(std::collections::HashSet::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .contains(&path)
        {
            return Err(Error::Corrupt(
                "injected schema v20 migration failure after DDL".into(),
            ));
        }
    }
    #[cfg(not(any(test, debug_assertions)))]
    let _ = conn;
    Ok(())
}

#[cfg(any(test, debug_assertions))]
pub(crate) fn create_schema_v18_fixture_for_test(workspace: &Path) -> Result<()> {
    Trail::init(workspace, "main", InitImportMode::Empty, false)?;
    let workspace = canonicalize_lossless(workspace)?;
    let db_path = workspace.join(DB_RELATIVE_PATH_WITH_TRAIL_PREFIX);
    let seed_path = workspace.join(".trail/index/trail-schema19-seed.sqlite");
    {
        let conn = Connection::open(&db_path)?;
        let busy: i64 = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |row| row.get(0))?;
        if busy != 0 {
            return Err(Error::Conflict(
                "schema-18 fixture seed checkpoint remained busy".into(),
            ));
        }
    }
    fs::rename(&db_path, &seed_path)?;
    let conn = Connection::open(&db_path)?;
    conn.execute_batch(PROLLY_SQLITE_SCHEMA)?;
    ddl::create_schema_v18_for_test(&conn)?;
    conn.execute(
        "ATTACH DATABASE ?1 AS seed",
        [seed_path.to_string_lossy().as_ref()],
    )?;
    let tables = conn
        .prepare(
            "SELECT name FROM seed.sqlite_master
             WHERE type='table' AND name NOT LIKE 'sqlite_%'
               AND name<>'schema_meta' AND name<>'lane_initializations'
               AND name IN (SELECT name FROM main.sqlite_master WHERE type='table')
             ORDER BY name",
        )?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    for table in tables {
        let quoted = table.replace('"', "\"\"");
        conn.execute_batch(&format!(
            "INSERT INTO main.\"{quoted}\" SELECT * FROM seed.\"{quoted}\";"
        ))?;
    }
    conn.execute_batch("DETACH DATABASE seed;")?;
    validate_schema_v18_for_migration(&conn)?;
    drop(conn);
    fs::remove_file(seed_path)?;
    Ok(())
}

#[cfg(any(test, debug_assertions))]
const DB_RELATIVE_PATH_WITH_TRAIL_PREFIX: &str = ".trail/index/trail.sqlite";

pub(crate) fn validate_prolly_sqlite_schema_v18(conn: &Connection) -> Result<()> {
    let expected = Connection::open_in_memory()?;
    expected.execute_batch(PROLLY_SQLITE_SCHEMA)?;
    if prolly_sqlite_objects(conn)? != prolly_sqlite_objects(&expected)?
        || prolly_sqlite_structure(conn)? != prolly_sqlite_structure(&expected)?
    {
        return Err(Error::Corrupt(
            "SQLite Prolly schema v18 shape is incomplete".into(),
        ));
    }
    Ok(())
}

fn prolly_sqlite_structure(conn: &Connection) -> Result<Vec<ProllySqliteTableStructure>> {
    let mut structure = Vec::new();
    for table in ["prolly_hints", "prolly_nodes", "prolly_roots"] {
        let columns = conn
            .prepare(
                "SELECT cid, name, type, [notnull], COALESCE(dflt_value, ''), pk, hidden
                 FROM pragma_table_xinfo(?1)
                 ORDER BY cid",
            )?
            .query_map([table], |row| {
                Ok(format!(
                    "{}|{}|{}|{}|{}|{}|{}",
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;

        let indexes = conn
            .prepare(
                "SELECT name FROM pragma_index_list(?1)
                 ORDER BY name",
            )?
            .query_map([table], |row| row.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .map(|index| {
                let entries = conn
                    .prepare(
                        "SELECT seqno, cid, COALESCE(name, ''), [desc], coll, key
                         FROM pragma_index_xinfo(?1)
                         ORDER BY seqno",
                    )?
                    .query_map([&index], |row| {
                        Ok(format!(
                            "{}|{}|{}|{}|{}|{}",
                            row.get::<_, i64>(0)?,
                            row.get::<_, i64>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, i64>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, i64>(5)?,
                        ))
                    })?
                    .collect::<std::result::Result<Vec<_>, _>>()?;
                Ok((index, entries))
            })
            .collect::<Result<Vec<_>>>()?;
        structure.push((table.to_string(), columns, indexes));
    }
    Ok(structure)
}

fn prolly_sqlite_objects(conn: &Connection) -> Result<Vec<(String, String, String)>> {
    let mut statement = conn.prepare(
        "SELECT type, name, sql FROM sqlite_master
         WHERE name LIKE 'prolly_%'
           AND name NOT LIKE 'sqlite_%'
         ORDER BY type, name",
    )?;
    let objects = statement
        .query_map([], |row| {
            let sql = row.get::<_, Option<String>>(2)?.unwrap_or_default();
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                sql.split_whitespace().collect::<Vec<_>>().join(" "),
            ))
        })?
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(Error::from)?;
    Ok(objects)
}
