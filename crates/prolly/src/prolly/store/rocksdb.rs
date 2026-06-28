//! RocksDB storage backend implementation

use std::collections::HashMap;
use std::path::Path;

use rocksdb::{DBCompressionType, Options, WriteBatch, DB};

use super::{BatchOp, Store};

/// Compression type for RocksDB
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CompressionType {
    /// No compression
    None,
    /// Snappy compression (fast, moderate compression)
    Snappy,
    /// Zlib compression (slower, better compression)
    Zlib,
    /// LZ4 compression (very fast, good compression)
    #[default]
    Lz4,
    /// LZ4HC compression (slower than LZ4, better compression)
    Lz4hc,
    /// Zstd compression (good balance of speed and compression)
    Zstd,
}

impl From<CompressionType> for DBCompressionType {
    fn from(ct: CompressionType) -> Self {
        match ct {
            CompressionType::None => DBCompressionType::None,
            CompressionType::Snappy => DBCompressionType::Snappy,
            CompressionType::Zlib => DBCompressionType::Zlib,
            CompressionType::Lz4 => DBCompressionType::Lz4,
            CompressionType::Lz4hc => DBCompressionType::Lz4hc,
            CompressionType::Zstd => DBCompressionType::Zstd,
        }
    }
}

/// Configuration options for RocksDBStore
#[derive(Debug, Clone)]
pub struct RocksDBConfig {
    /// Create database if it doesn't exist (default: true)
    pub create_if_missing: bool,
    /// Compression type (default: Lz4)
    pub compression: CompressionType,
    /// Block cache size in bytes (default: 64MB)
    pub cache_size: usize,
    /// Enable statistics collection (default: false)
    pub enable_statistics: bool,
}

impl Default for RocksDBConfig {
    fn default() -> Self {
        Self {
            create_if_missing: true,
            compression: CompressionType::Lz4,
            cache_size: 64 * 1024 * 1024, // 64MB
            enable_statistics: false,
        }
    }
}

/// Error type for RocksDB store operations
#[derive(Debug)]
pub struct RocksDBStoreError {
    message: String,
    source: Option<rocksdb::Error>,
}

impl RocksDBStoreError {
    /// Create a new error with a message
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            source: None,
        }
    }

    /// Create a new error from a RocksDB error
    pub fn from_rocksdb(err: rocksdb::Error, context: impl Into<String>) -> Self {
        Self {
            message: format!("{}: {}", context.into(), err),
            source: Some(err),
        }
    }
}

impl std::fmt::Display for RocksDBStoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "RocksDB error: {}", self.message)
    }
}

impl std::error::Error for RocksDBStoreError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|e| e as &(dyn std::error::Error + 'static))
    }
}

impl From<rocksdb::Error> for RocksDBStoreError {
    fn from(err: rocksdb::Error) -> Self {
        Self {
            message: err.to_string(),
            source: Some(err),
        }
    }
}

/// RocksDB-based storage backend for Prolly Trees
///
/// This store provides persistent key-value storage using RocksDB.
/// It is thread-safe (Send + Sync) and supports atomic batch operations.
pub struct RocksDBStore {
    db: DB,
}

impl RocksDBStore {
    /// Open or create a RocksDB database at the given path with default config
    ///
    /// # Arguments
    /// * `path` - Path to the database directory
    ///
    /// # Returns
    /// A new RocksDBStore instance or an error if the database cannot be opened
    ///
    /// # Example
    /// ```no_run
    /// use prolly::RocksDBStore;
    ///
    /// let store = RocksDBStore::open("/tmp/my_db").unwrap();
    /// ```
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, RocksDBStoreError> {
        Self::open_with_config(path, RocksDBConfig::default())
    }

    /// Open or create a RocksDB database with custom configuration
    ///
    /// # Arguments
    /// * `path` - Path to the database directory
    /// * `config` - Configuration options for the database
    ///
    /// # Returns
    /// A new RocksDBStore instance or an error if the database cannot be opened
    ///
    /// # Example
    /// ```no_run
    /// use prolly::{RocksDBStore, RocksDBConfig, CompressionType};
    ///
    /// let config = RocksDBConfig {
    ///     compression: CompressionType::Zstd,
    ///     cache_size: 128 * 1024 * 1024, // 128MB
    ///     ..Default::default()
    /// };
    /// let store = RocksDBStore::open_with_config("/tmp/my_db", config).unwrap();
    /// ```
    pub fn open_with_config<P: AsRef<Path>>(
        path: P,
        config: RocksDBConfig,
    ) -> Result<Self, RocksDBStoreError> {
        let mut opts = Options::default();

        // Apply configuration options
        opts.create_if_missing(config.create_if_missing);
        opts.set_compression_type(config.compression.into());

        // Set up block cache
        let mut block_opts = rocksdb::BlockBasedOptions::default();
        let cache = rocksdb::Cache::new_lru_cache(config.cache_size);
        block_opts.set_block_cache(&cache);
        opts.set_block_based_table_factory(&block_opts);

        // Enable statistics if requested
        if config.enable_statistics {
            opts.enable_statistics();
        }

        // Open the database
        let db = DB::open(&opts, path.as_ref()).map_err(|e| {
            RocksDBStoreError::from_rocksdb(
                e,
                format!("Failed to open database at {:?}", path.as_ref()),
            )
        })?;

        Ok(Self { db })
    }
}

impl Store for RocksDBStore {
    type Error = RocksDBStoreError;

    fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
        self.db
            .get(key)
            .map_err(|e| RocksDBStoreError::from_rocksdb(e, "Failed to read key"))
    }

    fn put(&self, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
        self.db
            .put(key, value)
            .map_err(|e| RocksDBStoreError::from_rocksdb(e, "Failed to write key"))
    }

    fn delete(&self, key: &[u8]) -> Result<(), Self::Error> {
        self.db
            .delete(key)
            .map_err(|e| RocksDBStoreError::from_rocksdb(e, "Failed to delete key"))
    }

    fn batch(&self, ops: &[BatchOp]) -> Result<(), Self::Error> {
        let mut batch = WriteBatch::default();

        for op in ops {
            match op {
                BatchOp::Upsert { key, value } => {
                    batch.put(key, value);
                }
                BatchOp::Delete { key } => {
                    batch.delete(key);
                }
            }
        }

        self.db
            .write(batch)
            .map_err(|e| RocksDBStoreError::from_rocksdb(e, "Batch operation failed"))
    }

    fn batch_get(&self, keys: &[&[u8]]) -> Result<HashMap<Vec<u8>, Vec<u8>>, Self::Error> {
        let results = self.db.multi_get(keys);
        let mut map = HashMap::with_capacity(keys.len());

        for (key, result) in keys.iter().zip(results) {
            match result {
                Ok(Some(value)) => {
                    map.insert(key.to_vec(), value);
                }
                Ok(None) => {
                    // Key not found, skip
                }
                Err(e) => {
                    return Err(RocksDBStoreError::from_rocksdb(
                        e,
                        "Failed to read key in batch",
                    ));
                }
            }
        }

        Ok(map)
    }

    fn batch_get_ordered(&self, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>, Self::Error> {
        let results = self.db.multi_get(keys);
        let mut ordered = Vec::with_capacity(keys.len());

        for result in results {
            match result {
                Ok(value) => {
                    ordered.push(value);
                }
                Err(e) => {
                    return Err(RocksDBStoreError::from_rocksdb(
                        e,
                        "Failed to read key in batch",
                    ));
                }
            }
        }

        Ok(ordered)
    }

    fn prefers_batch_reads(&self) -> bool {
        true
    }

    fn batch_put(&self, entries: &[(&[u8], &[u8])]) -> Result<(), Self::Error> {
        let mut batch = WriteBatch::default();

        for (key, value) in entries {
            batch.put(key, value);
        }

        self.db
            .write(batch)
            .map_err(|e| RocksDBStoreError::from_rocksdb(e, "Batch put operation failed"))
    }
}
