use super::*;
use std::ffi::{c_int, c_void};
use std::sync::OnceLock;

use rusqlite::ffi::{
    sqlite3_auto_extension, sqlite3_file_control, SQLITE_FCNTL_PERSIST_WAL, SQLITE_OK,
};
use sqlite_vec::sqlite3_vec_init;

static SQLITE_VEC_REGISTRATION: OnceLock<i32> = OnceLock::new();

pub(crate) fn register_sqlite_vec_extension() -> Result<()> {
    let result = *SQLITE_VEC_REGISTRATION.get_or_init(|| {
        // sqlite-vec exposes a C extension entrypoint. Register it once before
        // opening connections that may read vec0 virtual tables.
        unsafe {
            sqlite3_auto_extension(Some(std::mem::transmute::<
                *const (),
                unsafe extern "C" fn(
                    *mut rusqlite::ffi::sqlite3,
                    *mut *const i8,
                    *const rusqlite::ffi::sqlite3_api_routines,
                ) -> i32,
            >(sqlite3_vec_init as *const ())))
        }
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
    // WAL mode persists in the database header. Reissuing the mutating
    // journal-mode pragma on every observer control connection can require an
    // exclusive transition while the daemon's primary connection and WAL are
    // live; native Linux filesystems can report that failed transition as
    // IOERR. New databases still opt into WAL, while runtime connections only
    // verify and reuse the already-persisted mode.
    let journal_mode: String = conn
        .query_row("PRAGMA journal_mode", [], |row| row.get(0))
        .map_err(|error| {
            Error::DaemonUnavailable(format!("SQLite journal-mode verification failed: {error}"))
        })?;
    if !journal_mode.eq_ignore_ascii_case("wal") {
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|error| {
                Error::DaemonUnavailable(format!("SQLite WAL-mode initialization failed: {error}"))
            })?;
    }
    conn.pragma_update(None, "synchronous", "NORMAL")
        .map_err(|error| {
            Error::DaemonUnavailable(format!(
                "SQLite synchronous-mode initialization failed: {error}"
            ))
        })?;
    apply_sqlite_runtime_pragmas(conn)
}

/// Configure connection-local behavior without inspecting or mutating the
/// database-wide journal mode. Runtime observer connections open only after
/// Trail's hard-cutover initializer has established WAL mode.
pub(crate) fn apply_sqlite_runtime_pragmas(conn: &Connection) -> Result<()> {
    persist_wal_across_process_lifetimes(conn)?;
    // Do not lower `synchronous` on a live database. New SQLite connections
    // default to FULL, which is stronger than Trail's initialized NORMAL mode;
    // attempting to change it while the primary daemon transaction is live can
    // fail with IOERR on native Linux.
    conn.pragma_update(None, "foreign_keys", "ON")
        .map_err(|error| {
            Error::DaemonUnavailable(format!("SQLite foreign-key initialization failed: {error}"))
        })?;
    conn.pragma_update(None, "temp_store", "MEMORY")
        .map_err(|error| {
            Error::DaemonUnavailable(format!(
                "SQLite temporary-store initialization failed: {error}"
            ))
        })?;
    Ok(())
}

fn persist_wal_across_process_lifetimes(conn: &Connection) -> Result<()> {
    let mut enabled: c_int = 1;
    // SAFETY: `conn` remains alive for the call, `main` is NUL terminated,
    // and SQLite reads/writes one `int` for SQLITE_FCNTL_PERSIST_WAL.
    let result = unsafe {
        sqlite3_file_control(
            conn.handle(),
            c"main".as_ptr(),
            SQLITE_FCNTL_PERSIST_WAL,
            (&mut enabled as *mut c_int).cast::<c_void>(),
        )
    };
    if result == SQLITE_OK {
        Ok(())
    } else {
        Err(Error::DaemonUnavailable(format!(
            "SQLite persistent-WAL configuration failed with result code {result}"
        )))
    }
}
