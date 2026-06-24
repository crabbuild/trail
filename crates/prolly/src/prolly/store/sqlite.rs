//! SQLite storage backend implementation

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use rusqlite::{params, Connection, OptionalExtension};

use super::{BatchOp, Store};

const CREATE_TABLE_SQL: &str = "\
CREATE TABLE IF NOT EXISTS prolly_nodes (
    cid  BLOB PRIMARY KEY NOT NULL,
    node BLOB NOT NULL
) WITHOUT ROWID;";

const SELECT_SQL: &str = "SELECT node FROM prolly_nodes WHERE cid = ?1";
const UPSERT_SQL: &str = "\
INSERT INTO prolly_nodes (cid, node)
VALUES (?1, ?2)
ON CONFLICT(cid) DO UPDATE SET node = excluded.node";
const DELETE_SQL: &str = "DELETE FROM prolly_nodes WHERE cid = ?1";

/// Configuration options for [`SqliteStore`].
#[derive(Debug, Clone)]
pub struct SqliteStoreConfig {
    /// Busy timeout in milliseconds for contended SQLite locks.
    pub busy_timeout_ms: u64,
    /// Enable WAL journaling for file-backed databases.
    pub enable_wal: bool,
    /// Set SQLite synchronous mode to NORMAL when applying default pragmas.
    pub synchronous_normal: bool,
}

impl Default for SqliteStoreConfig {
    fn default() -> Self {
        Self {
            busy_timeout_ms: 5_000,
            enable_wal: true,
            synchronous_normal: true,
        }
    }
}

/// Error type for SQLite store operations.
#[derive(Debug)]
pub struct SqliteStoreError {
    message: String,
    source: Option<rusqlite::Error>,
}

impl SqliteStoreError {
    /// Create a new error with a message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// Create a new error from a rusqlite error.
    pub fn from_sqlite(err: rusqlite::Error, context: impl Into<String>) -> Self {
        Self {
            message: format!("{}: {}", context.into(), err),
            source: Some(err),
        }
    }
}

impl std::fmt::Display for SqliteStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SQLite error: {}", self.message)
    }
}

impl std::error::Error for SqliteStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

impl From<rusqlite::Error> for SqliteStoreError {
    fn from(err: rusqlite::Error) -> Self {
        Self {
            message: err.to_string(),
            source: Some(err),
        }
    }
}

/// SQLite-backed storage backend for Prolly Trees.
///
/// This store persists content-addressed nodes in a single SQLite table and
/// supports atomic batch operations through transactions.
pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    /// Open or create a SQLite database at the given path with default config.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, SqliteStoreError> {
        Self::open_with_config(path, SqliteStoreConfig::default())
    }

    /// Open or create a SQLite database with custom configuration.
    pub fn open_with_config<P: AsRef<Path>>(
        path: P,
        config: SqliteStoreConfig,
    ) -> Result<Self, SqliteStoreError> {
        let conn = Connection::open(path.as_ref()).map_err(|e| {
            SqliteStoreError::from_sqlite(
                e,
                format!("Failed to open database at {:?}", path.as_ref()),
            )
        })?;
        Self::from_connection(conn, config)
    }

    /// Create an in-memory SQLite store.
    pub fn open_in_memory() -> Result<Self, SqliteStoreError> {
        let conn = Connection::open_in_memory()
            .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to open in-memory database"))?;
        Self::from_connection(conn, SqliteStoreConfig::default())
    }

    fn from_connection(
        conn: Connection,
        config: SqliteStoreConfig,
    ) -> Result<Self, SqliteStoreError> {
        conn.busy_timeout(Duration::from_millis(config.busy_timeout_ms))
            .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to set busy timeout"))?;

        if config.enable_wal {
            conn.pragma_update(None, "journal_mode", "WAL")
                .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to enable WAL mode"))?;
        }
        if config.synchronous_normal {
            conn.pragma_update(None, "synchronous", "NORMAL")
                .map_err(|e| {
                    SqliteStoreError::from_sqlite(e, "Failed to set synchronous=NORMAL")
                })?;
        }
        conn.pragma_update(None, "temp_store", "MEMORY")
            .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to set temp_store=MEMORY"))?;
        conn.execute_batch(CREATE_TABLE_SQL)
            .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to initialize schema"))?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn connection(&self) -> Result<MutexGuard<'_, Connection>, SqliteStoreError> {
        self.conn
            .lock()
            .map_err(|e| SqliteStoreError::new(format!("lock poisoned: {}", e)))
    }
}

impl Store for SqliteStore {
    type Error = SqliteStoreError;

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
        let conn = self.connection()?;
        conn.query_row(SELECT_SQL, params![key], |row| row.get(0))
            .optional()
            .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to read key"))
    }

    fn put(&self, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        let conn = self.connection()?;
        conn.execute(UPSERT_SQL, params![key, value])
            .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to write key"))?;
        Ok(())
    }

    fn delete(&self, key: &[u8]) -> Result<(), Self::Error> {
        let conn = self.connection()?;
        conn.execute(DELETE_SQL, params![key])
            .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to delete key"))?;
        Ok(())
    }

    fn batch(&self, ops: &[BatchOp]) -> Result<(), Self::Error> {
        let mut conn = self.connection()?;
        let tx = conn
            .transaction()
            .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to start transaction"))?;

        for op in ops {
            match op {
                BatchOp::Upsert { key, value } => {
                    tx.execute(UPSERT_SQL, params![key, value]).map_err(|e| {
                        SqliteStoreError::from_sqlite(e, "Failed to write key in batch")
                    })?;
                }
                BatchOp::Delete { key } => {
                    tx.execute(DELETE_SQL, params![key]).map_err(|e| {
                        SqliteStoreError::from_sqlite(e, "Failed to delete key in batch")
                    })?;
                }
            }
        }

        tx.commit()
            .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to commit transaction"))
    }

    fn batch_get(&self, keys: &[&[u8]]) -> Result<HashMap<Vec<u8>, Vec<u8>>, Self::Error> {
        let conn = self.connection()?;
        let mut stmt = conn
            .prepare_cached(SELECT_SQL)
            .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to prepare batch read"))?;
        let mut results = HashMap::with_capacity(keys.len());

        for key in keys {
            if let Some(value) = stmt
                .query_row(params![key], |row| row.get(0))
                .optional()
                .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to read key in batch"))?
            {
                results.insert(key.to_vec(), value);
            }
        }

        Ok(results)
    }

    fn batch_get_ordered(&self, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>, Self::Error> {
        let conn = self.connection()?;
        let mut stmt = conn.prepare_cached(SELECT_SQL).map_err(|e| {
            SqliteStoreError::from_sqlite(e, "Failed to prepare ordered batch read")
        })?;
        let mut results = Vec::with_capacity(keys.len());

        for key in keys {
            let value = stmt
                .query_row(params![key], |row| row.get(0))
                .optional()
                .map_err(|e| {
                    SqliteStoreError::from_sqlite(e, "Failed to read key in ordered batch")
                })?;
            results.push(value);
        }

        Ok(results)
    }

    fn batch_put(&self, entries: &[(&[u8], &[u8])]) -> Result<(), Self::Error> {
        let mut conn = self.connection()?;
        let tx = conn
            .transaction()
            .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to start transaction"))?;

        for (key, value) in entries {
            tx.execute(UPSERT_SQL, params![key, value]).map_err(|e| {
                SqliteStoreError::from_sqlite(e, "Failed to write key in batch_put")
            })?;
        }

        tx.commit()
            .map_err(|e| SqliteStoreError::from_sqlite(e, "Failed to commit transaction"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sqlite_store_put_get_delete() {
        let store = SqliteStore::open_in_memory().unwrap();

        store.put(b"key", b"value").unwrap();
        assert_eq!(store.get(b"key").unwrap(), Some(b"value".to_vec()));

        store.delete(b"key").unwrap();
        assert_eq!(store.get(b"key").unwrap(), None);
    }

    #[test]
    fn sqlite_store_batch_is_order_preserving_for_reads() {
        let store = SqliteStore::open_in_memory().unwrap();
        let ops = vec![
            BatchOp::Upsert {
                key: b"a",
                value: b"1",
            },
            BatchOp::Upsert {
                key: b"b",
                value: b"2",
            },
            BatchOp::Upsert {
                key: b"c",
                value: b"3",
            },
        ];

        store.batch(&ops).unwrap();

        let keys: Vec<&[u8]> = vec![b"c", b"missing", b"a", b"b"];
        assert_eq!(
            store.batch_get_ordered(&keys).unwrap(),
            vec![
                Some(b"3".to_vec()),
                None,
                Some(b"1".to_vec()),
                Some(b"2".to_vec())
            ]
        );
    }

    #[test]
    fn sqlite_store_batch_put_updates_existing_keys() {
        let store = SqliteStore::open_in_memory().unwrap();

        store.put(b"a", b"old").unwrap();
        store
            .batch_put(&[(b"a".as_slice(), b"new".as_slice()), (b"b", b"2")])
            .unwrap();

        assert_eq!(store.get(b"a").unwrap(), Some(b"new".to_vec()));
        assert_eq!(store.get(b"b").unwrap(), Some(b"2".to_vec()));
    }
}
