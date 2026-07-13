use super::*;

mod agent_capture;
mod changed_path_ledger;
mod ddl;
mod version;

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
    pub(crate) fn validate_schema_v18(conn: &Connection) -> Result<()> {
        validate_schema_v18(conn)
    }
}

pub(crate) fn validate_schema_v18(conn: &Connection) -> Result<()> {
    let user_version = conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))?;
    if user_version != TRAIL_SCHEMA_VERSION {
        return Err(Error::Corrupt(format!(
            "found version {user_version}; expected version {TRAIL_SCHEMA_VERSION}"
        )));
    }
    if !ddl::base_schema_v18_complete(conn)? {
        return Err(Error::Corrupt("base schema v18 shape is incomplete".into()));
    }
    if !agent_capture::agent_capture_schema_complete(conn)? {
        return Err(Error::Corrupt(
            "agent capture schema v18 shape is incomplete".into(),
        ));
    }
    if !changed_path_ledger::changed_path_ledger_schema_complete(conn)? {
        return Err(Error::Corrupt(
            "changed-path ledger schema v18 shape is incomplete".into(),
        ));
    }
    Ok(())
}

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

fn prolly_sqlite_structure(
    conn: &Connection,
) -> Result<Vec<(String, Vec<String>, Vec<(String, Vec<String>)>)>> {
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

pub(crate) fn validate_no_prolly_sqlite_schema_v18(conn: &Connection) -> Result<()> {
    if !prolly_sqlite_objects(conn)?.is_empty() {
        return Err(Error::Corrupt(
            "SlateDB workspace contains unexpected SQLite Prolly objects".into(),
        ));
    }
    Ok(())
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
