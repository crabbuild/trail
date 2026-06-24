//! Storage backend trait and implementations for Prolly Trees

mod memory;
#[cfg(feature = "rocksdb")]
mod rocksdb;
#[cfg(feature = "sqlite")]
mod sqlite;

#[cfg(feature = "rocksdb")]
pub use self::rocksdb::{CompressionType, RocksDBConfig, RocksDBStore, RocksDBStoreError};
#[cfg(feature = "sqlite")]
pub use self::sqlite::{SqliteStore, SqliteStoreConfig, SqliteStoreError};
pub use memory::{MemStore, MemStoreError};

use std::collections::HashMap;

/// Batch operation for atomic writes
#[derive(Debug, Clone)]
pub enum BatchOp<'a> {
    /// Insert or update a key-value pair
    Upsert { key: &'a [u8], value: &'a [u8] },
    /// Delete a key
    Delete { key: &'a [u8] },
}

/// Storage backend trait for Prolly Trees
///
/// Keys are CID bytes, values are serialized nodes.
/// Implementations must be thread-safe (Send + Sync).
pub trait Store: Send + Sync {
    /// Error type for storage operations
    type Error: std::error::Error + Send + Sync + 'static;

    /// Get value by key
    ///
    /// Returns `Ok(Some(value))` if key exists, `Ok(None)` if not found,
    /// or `Err` on storage failure.
    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error>;

    /// Store key-value pair
    ///
    /// Inserts or updates the value for the given key.
    fn put(&self, key: &[u8], value: &[u8]) -> Result<(), Self::Error>;

    /// Delete key
    ///
    /// Removes the key if it exists. No error if key doesn't exist.
    fn delete(&self, key: &[u8]) -> Result<(), Self::Error>;

    /// Batch write operations (atomic if supported by backend)
    ///
    /// Applies all operations in the batch. Implementations should
    /// attempt to make this atomic when possible.
    fn batch(&self, ops: &[BatchOp]) -> Result<(), Self::Error>;

    /// Retrieve multiple keys in a single operation
    ///
    /// Returns a HashMap mapping each requested key to its value (if found).
    /// Keys that don't exist are simply not included in the result.
    ///
    /// The default implementation uses sequential gets, but implementations
    /// can override this for better performance.
    fn batch_get(&self, keys: &[&[u8]]) -> Result<HashMap<Vec<u8>, Vec<u8>>, Self::Error> {
        let mut results = HashMap::new();
        for key in keys {
            if let Some(value) = self.get(key)? {
                results.insert(key.to_vec(), value);
            }
        }
        Ok(results)
    }

    /// Retrieve multiple keys in a single operation with order preservation
    ///
    /// Returns a Vec of `Option<Vec<u8>>` in the same order as the input keys.
    /// Each element is `Some(value)` if the key exists, or `None` if not found.
    ///
    /// This method is useful when the order of results must match the order
    /// of input keys, such as when prefetching nodes for batch operations.
    ///
    /// The default implementation uses sequential gets, but implementations
    /// with parallel I/O capabilities (e.g., network stores) can override
    /// this for better performance.
    ///
    /// # Arguments
    /// * `keys` - Slice of keys to retrieve
    ///
    /// # Returns
    /// Vector of `Option<Vec<u8>>` in the same order as input keys.
    /// `None` indicates the key was not found.
    fn batch_get_ordered(&self, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>, Self::Error> {
        let mut results = Vec::with_capacity(keys.len());
        for key in keys {
            results.push(self.get(key)?);
        }
        Ok(results)
    }

    /// Store multiple key-value pairs in a single operation
    ///
    /// Writes all entries atomically when possible. The default implementation
    /// uses the existing batch method with Upsert operations.
    ///
    /// Implementations can override this for better performance.
    fn batch_put(&self, entries: &[(&[u8], &[u8])]) -> Result<(), Self::Error> {
        let ops: Vec<BatchOp> = entries
            .iter()
            .map(|(k, v)| BatchOp::Upsert { key: k, value: v })
            .collect();
        self.batch(&ops)
    }
}

/// Implement Store for `Arc<T>` where T: Store
/// This allows sharing a store between multiple Prolly instances
impl<T: Store> Store for std::sync::Arc<T> {
    type Error = T::Error;

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
        (**self).get(key)
    }

    fn put(&self, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        (**self).put(key, value)
    }

    fn delete(&self, key: &[u8]) -> Result<(), Self::Error> {
        (**self).delete(key)
    }

    fn batch(&self, ops: &[BatchOp]) -> Result<(), Self::Error> {
        (**self).batch(ops)
    }

    fn batch_get(&self, keys: &[&[u8]]) -> Result<HashMap<Vec<u8>, Vec<u8>>, Self::Error> {
        (**self).batch_get(keys)
    }

    fn batch_get_ordered(&self, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>, Self::Error> {
        (**self).batch_get_ordered(keys)
    }

    fn batch_put(&self, entries: &[(&[u8], &[u8])]) -> Result<(), Self::Error> {
        (**self).batch_put(entries)
    }
}
