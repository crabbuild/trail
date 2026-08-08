use super::*;
use crate::db::storage::validate_prolly_sqlite_schema;

mod create;
mod publication;
mod restore;
mod restore_transaction;
mod verify;

pub(crate) use restore_transaction::recover_restore_publication;

/// Open a private backup/restore staging tree after validating its SQLite schema.
///
/// Staged trees are created by Trail itself and are not exposed to concurrent
/// workspace writers. Validate the copied database while holding its workspace
/// lock, then use the write-lock handoff entry point so SQLite may initialize
/// its platform-specific WAL/SHM runtime files without being mistaken for a
/// replacement of the staged database.
pub(super) fn open_staged_copy(workspace_root: &Path, db_dir: &Path) -> Result<Trail> {
    let _lock = acquire_workspace_lock(db_dir)?;
    let sqlite_path = db_dir.join(DB_RELATIVE_PATH);
    let conn = Connection::open(&sqlite_path)?;
    conn.pragma_update(None, "foreign_keys", true)?;
    Trail::validate_schema(&conn)?;
    validate_prolly_sqlite_schema(&conn)?;
    drop(conn);
    Trail::open_without_recovering_derived_paths_under_write_lock(workspace_root, db_dir)
}
