use super::*;

mod agent_capture;
mod changed_path_ledger;
mod ddl;
mod version;

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
