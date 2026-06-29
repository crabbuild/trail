//! Storage backend trait and implementations for Prolly Trees

mod memory;
#[cfg(feature = "pglite")]
mod pglite;
#[cfg(feature = "rocksdb")]
mod rocksdb;
#[cfg(feature = "slatedb")]
mod slatedb;
#[cfg(feature = "sqlite")]
mod sqlite;

#[cfg(feature = "pglite")]
pub use self::pglite::{PgliteStore, PgliteStoreConfig, PgliteStoreError};
#[cfg(feature = "rocksdb")]
pub use self::rocksdb::{CompressionType, RocksDBConfig, RocksDBStore, RocksDBStoreError};
#[cfg(feature = "slatedb")]
pub use self::slatedb::{SlateDbStore, SlateDbStoreConfig, SlateDbStoreError};
#[cfg(feature = "sqlite")]
pub use self::sqlite::{SqliteStore, SqliteStoreConfig, SqliteStoreError};
pub use memory::{MemStore, MemStoreError};

use std::collections::{hash_map::Entry, HashMap};

pub(crate) struct OrderedBatchReadPlan<'a> {
    unique_keys: Vec<&'a [u8]>,
    positions: Option<Vec<usize>>,
}

impl<'a> OrderedBatchReadPlan<'a> {
    pub(crate) fn new(keys: &[&'a [u8]]) -> Self {
        if keys.len() < 2 {
            return Self {
                unique_keys: keys.to_vec(),
                positions: None,
            };
        }

        let mut unique_indexes = HashMap::with_capacity(keys.len());
        let mut unique_keys = Vec::with_capacity(keys.len());
        let mut positions: Option<Vec<usize>> = None;

        for key in keys {
            match unique_indexes.entry(*key) {
                Entry::Occupied(entry) => {
                    let positions =
                        positions.get_or_insert_with(|| (0..unique_keys.len()).collect());
                    positions.push(*entry.get());
                }
                Entry::Vacant(entry) => {
                    let unique_idx = unique_keys.len();
                    unique_keys.push(*key);
                    if let Some(positions) = positions.as_mut() {
                        positions.push(unique_idx);
                    }
                    entry.insert(unique_idx);
                }
            }
        }

        Self {
            unique_keys,
            positions,
        }
    }

    pub(crate) fn unique_keys(&self) -> &[&'a [u8]] {
        &self.unique_keys
    }

    #[cfg(test)]
    pub(crate) fn is_identity(&self) -> bool {
        self.positions.is_none()
    }

    #[cfg(test)]
    pub(crate) fn expand<T: Clone>(&self, unique_values: &[Option<T>]) -> Vec<Option<T>> {
        debug_assert_eq!(self.unique_keys.len(), unique_values.len());
        match &self.positions {
            Some(positions) => positions
                .iter()
                .map(|&unique_idx| unique_values[unique_idx].clone())
                .collect(),
            None => unique_values.to_vec(),
        }
    }

    pub(crate) fn expand_owned<T: Clone>(&self, unique_values: Vec<Option<T>>) -> Vec<Option<T>> {
        debug_assert_eq!(self.unique_keys.len(), unique_values.len());
        match &self.positions {
            Some(positions) => positions
                .iter()
                .map(|&unique_idx| unique_values[unique_idx].clone())
                .collect(),
            None => unique_values,
        }
    }
}

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
        let plan = OrderedBatchReadPlan::new(keys);
        let mut results = HashMap::with_capacity(plan.unique_keys().len());
        for key in plan.unique_keys() {
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
        if keys.len() < 2 {
            return keys.iter().map(|key| self.get(key)).collect();
        }

        let plan = OrderedBatchReadPlan::new(keys);
        let mut unique_values = Vec::with_capacity(plan.unique_keys().len());
        for key in plan.unique_keys() {
            unique_values.push(self.get(key)?);
        }
        Ok(plan.expand_owned(unique_values))
    }

    /// Retrieve unique keys in input order.
    ///
    /// This is a fast path for callers that have already deduplicated keys and
    /// still need order preservation. The default keeps efficient custom
    /// `batch_get_ordered` implementations for stores that prefer batched
    /// reads, while avoiding duplicate-planning overhead for point-read stores.
    fn batch_get_ordered_unique(
        &self,
        keys: &[&[u8]],
    ) -> Result<Vec<Option<Vec<u8>>>, Self::Error> {
        if keys.is_empty() {
            return Ok(Vec::new());
        }

        if !self.prefers_batch_reads() {
            return keys.iter().map(|key| self.get(key)).collect();
        }

        self.batch_get_ordered(keys)
    }

    /// Whether this store has an efficient batched-read implementation.
    ///
    /// The prolly engine uses this to decide whether to prefetch many tree
    /// paths through `batch_get_ordered`. Stores that implement true multi-get,
    /// request coalescing, or parallel remote reads should return `true`.
    fn prefers_batch_reads(&self) -> bool {
        false
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

    /// Whether this store persists performance hints.
    fn supports_hints(&self) -> bool {
        false
    }

    /// Retrieve an optional performance hint for a logical namespace and key.
    ///
    /// Hints are not part of the content-addressed tree semantics. Store
    /// implementations may ignore them and return `None`; callers must always
    /// have a correct fallback path.
    fn get_hint(&self, namespace: &[u8], key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
        let _ = (namespace, key);
        Ok(None)
    }

    /// Persist an optional performance hint for a logical namespace and key.
    ///
    /// The default implementation is a no-op so custom stores remain compatible.
    fn put_hint(&self, namespace: &[u8], key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        let _ = (namespace, key, value);
        Ok(())
    }

    /// Store content-addressed nodes and one hint atomically when supported.
    fn batch_put_with_hint(
        &self,
        entries: &[(&[u8], &[u8])],
        namespace: &[u8],
        key: &[u8],
        value: &[u8],
    ) -> Result<(), Self::Error> {
        self.batch_put(entries)?;
        self.put_hint(namespace, key, value)
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

    fn batch_get_ordered_unique(
        &self,
        keys: &[&[u8]],
    ) -> Result<Vec<Option<Vec<u8>>>, Self::Error> {
        (**self).batch_get_ordered_unique(keys)
    }

    fn prefers_batch_reads(&self) -> bool {
        (**self).prefers_batch_reads()
    }

    fn batch_put(&self, entries: &[(&[u8], &[u8])]) -> Result<(), Self::Error> {
        (**self).batch_put(entries)
    }

    fn supports_hints(&self) -> bool {
        (**self).supports_hints()
    }

    fn get_hint(&self, namespace: &[u8], key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
        (**self).get_hint(namespace, key)
    }

    fn put_hint(&self, namespace: &[u8], key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        (**self).put_hint(namespace, key, value)
    }

    fn batch_put_with_hint(
        &self,
        entries: &[(&[u8], &[u8])],
        namespace: &[u8],
        key: &[u8],
        value: &[u8],
    ) -> Result<(), Self::Error> {
        (**self).batch_put_with_hint(entries, namespace, key, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    #[derive(Debug)]
    struct DefaultReadStoreError;

    impl std::fmt::Display for DefaultReadStoreError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("default read store error")
        }
    }

    impl std::error::Error for DefaultReadStoreError {}

    #[derive(Default)]
    struct DefaultReadStore {
        data: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
        get_calls: AtomicUsize,
    }

    impl DefaultReadStore {
        fn with_entries(entries: &[(&[u8], &[u8])]) -> Self {
            let mut data = BTreeMap::new();
            for (key, value) in entries {
                data.insert(key.to_vec(), value.to_vec());
            }

            Self {
                data: Mutex::new(data),
                get_calls: AtomicUsize::new(0),
            }
        }
    }

    impl Store for DefaultReadStore {
        type Error = DefaultReadStoreError;

        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
            self.get_calls.fetch_add(1, Ordering::Relaxed);
            Ok(self.data.lock().unwrap().get(key).cloned())
        }

        fn put(&self, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
            self.data
                .lock()
                .unwrap()
                .insert(key.to_vec(), value.to_vec());
            Ok(())
        }

        fn delete(&self, key: &[u8]) -> Result<(), Self::Error> {
            self.data.lock().unwrap().remove(key);
            Ok(())
        }

        fn batch(&self, ops: &[BatchOp]) -> Result<(), Self::Error> {
            let mut data = self.data.lock().unwrap();
            for op in ops {
                match op {
                    BatchOp::Upsert { key, value } => {
                        data.insert(key.to_vec(), value.to_vec());
                    }
                    BatchOp::Delete { key } => {
                        data.remove(*key);
                    }
                }
            }
            Ok(())
        }
    }

    #[test]
    fn ordered_batch_read_plan_keeps_unique_batches_identity() {
        let keys: Vec<&[u8]> = vec![b"a", b"b", b"missing"];
        let plan = OrderedBatchReadPlan::new(&keys);

        assert!(plan.is_identity());
        assert_eq!(
            plan.unique_keys(),
            &[b"a".as_slice(), b"b".as_slice(), b"missing".as_slice()]
        );

        let values = vec![Some(b"1".to_vec()), Some(b"2".to_vec()), None];
        let values_ptr = values.as_ptr();
        let expanded = plan.expand_owned(values);

        assert_eq!(expanded.as_ptr(), values_ptr);
        assert_eq!(
            expanded,
            vec![Some(b"1".to_vec()), Some(b"2".to_vec()), None]
        );
    }

    #[test]
    fn ordered_batch_read_plan_deduplicates_and_expands_slots() {
        let keys: Vec<&[u8]> = vec![b"c", b"a", b"c", b"missing", b"missing", b"a"];
        let plan = OrderedBatchReadPlan::new(&keys);

        assert!(!plan.is_identity());
        assert_eq!(
            plan.unique_keys(),
            &[b"c".as_slice(), b"a".as_slice(), b"missing".as_slice()]
        );
        assert_eq!(
            plan.expand(&[Some(b"3".to_vec()), Some(b"1".to_vec()), None]),
            vec![
                Some(b"3".to_vec()),
                Some(b"1".to_vec()),
                Some(b"3".to_vec()),
                None,
                None,
                Some(b"1".to_vec())
            ]
        );
        assert_eq!(
            plan.expand_owned(vec![Some(b"3".to_vec()), Some(b"1".to_vec()), None]),
            vec![
                Some(b"3".to_vec()),
                Some(b"1".to_vec()),
                Some(b"3".to_vec()),
                None,
                None,
                Some(b"1".to_vec())
            ]
        );
    }

    #[test]
    fn default_batch_get_deduplicates_duplicate_keys() {
        let store = DefaultReadStore::with_entries(&[(b"a", b"1"), (b"b", b"2")]);
        let keys: Vec<&[u8]> = vec![b"a", b"a", b"missing", b"missing", b"b"];

        let values = store.batch_get(&keys).unwrap();

        assert_eq!(values.get(b"a".as_slice()), Some(&b"1".to_vec()));
        assert_eq!(values.get(b"b".as_slice()), Some(&b"2".to_vec()));
        assert!(!values.contains_key(b"missing".as_slice()));
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            3,
            "default batch_get should point-read each unique key at most once"
        );
    }

    #[test]
    fn default_batch_get_ordered_deduplicates_while_preserving_slots() {
        let store = DefaultReadStore::with_entries(&[(b"a", b"1"), (b"b", b"2")]);
        let keys: Vec<&[u8]> = vec![b"a", b"a", b"missing", b"missing", b"b"];

        let values = store.batch_get_ordered(&keys).unwrap();

        assert_eq!(
            values,
            vec![
                Some(b"1".to_vec()),
                Some(b"1".to_vec()),
                None,
                None,
                Some(b"2".to_vec())
            ]
        );
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            3,
            "default ordered batch reads should preserve duplicate result slots without duplicate point reads"
        );
    }

    #[test]
    fn default_unique_ordered_batch_reads_preserve_order_with_point_reads() {
        let store = DefaultReadStore::with_entries(&[(b"a", b"1"), (b"b", b"2")]);
        let keys: Vec<&[u8]> = vec![b"b", b"missing", b"a"];

        let values = store.batch_get_ordered_unique(&keys).unwrap();

        assert_eq!(values, vec![Some(b"2".to_vec()), None, Some(b"1".to_vec())]);
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            3,
            "unique ordered batch reads for point-read stores should read each requested key once"
        );
    }
}
