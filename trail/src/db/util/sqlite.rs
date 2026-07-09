use super::*;
use std::sync::OnceLock;

use rusqlite::ffi::{sqlite3_auto_extension, SQLITE_OK};
use sqlite_vec::sqlite3_vec_init;

static SQLITE_VEC_REGISTRATION: OnceLock<i32> = OnceLock::new();

pub(crate) fn register_sqlite_vec_extension() -> Result<()> {
    let result = *SQLITE_VEC_REGISTRATION.get_or_init(|| {
        // sqlite-vec exposes a C extension entrypoint. Register it once before
        // opening connections that may read vec0 virtual tables.
        unsafe { sqlite3_auto_extension(Some(std::mem::transmute(sqlite3_vec_init as *const ()))) }
    });
    if result == SQLITE_OK {
        Ok(())
    } else {
        Err(Error::InvalidInput(format!(
            "failed to register sqlite-vec extension: sqlite result code {result}"
        )))
    }
}

pub(crate) fn apply_sqlite_pragmas(conn: &Connection) -> Result<()> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.pragma_update(None, "temp_store", "MEMORY")?;
    Ok(())
}

pub(crate) fn ensure_column(
    conn: &Connection,
    table: &'static str,
    column: &'static str,
    definition: &'static str,
) -> Result<()> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let columns = stmt
        .query_map([], |row| row.get::<_, String>(1))?
        .collect::<std::result::Result<Vec<_>, _>>()?;
    if !columns.iter().any(|existing| existing == column) {
        conn.execute(
            &format!("ALTER TABLE {table} ADD COLUMN {column} {definition}"),
            [],
        )?;
    }
    Ok(())
}
