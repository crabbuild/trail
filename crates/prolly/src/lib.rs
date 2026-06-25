//! # Prolly Trees
//!
//! A Rust implementation of Prolly Trees - content-addressable ordered search indexes
//! that combine the efficiency of B+ trees with deterministic merging capabilities.
//!
//! ## Features
//!
//! - **Ordered key-value storage**: Keys are sorted lexicographically (byte comparison)
//! - **Content-addressable nodes**: Each node has a unique CID (Content Identifier) derived from its content
//! - **Deterministic structure**: Same content always produces the same tree structure
//! - **Efficient diff/merge**: Compare trees by comparing root hashes, skip identical subtrees
//! - **Pluggable storage**: Implement the [`Store`] trait for custom backends
//!
//! ## Quick Start
//!
//! ```rust
//! use prolly::{Prolly, MemStore, Config};
//!
//! // Create a store and tree manager
//! let store = MemStore::new();
//! let prolly = Prolly::new(store, Config::default());
//!
//! // Create an empty tree
//! let tree = prolly.create();
//!
//! // Insert key-value pairs (returns a new tree - immutable)
//! let tree = prolly.put(&tree, b"name".to_vec(), b"Alice".to_vec()).unwrap();
//! let tree = prolly.put(&tree, b"age".to_vec(), b"30".to_vec()).unwrap();
//!
//! // Retrieve values
//! let name = prolly.get(&tree, b"name").unwrap();
//! assert_eq!(name, Some(b"Alice".to_vec()));
//!
//! // Delete keys
//! let tree = prolly.delete(&tree, b"age").unwrap();
//! assert!(prolly.get(&tree, b"age").unwrap().is_none());
//! ```
//!
//! ## Range Iteration
//!
//! ```rust
//! use prolly::{Prolly, MemStore, Config};
//!
//! let store = MemStore::new();
//! let prolly = Prolly::new(store, Config::default());
//! let mut tree = prolly.create();
//!
//! // Insert some data
//! tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
//! tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();
//! tree = prolly.put(&tree, b"c".to_vec(), b"3".to_vec()).unwrap();
//!
//! // Iterate over all keys
//! for result in prolly.range(&tree, &[], None).unwrap() {
//!     let (key, val) = result.unwrap();
//!     println!("{:?} -> {:?}", String::from_utf8_lossy(&key), String::from_utf8_lossy(&val));
//! }
//!
//! // Iterate over a specific range [b, c)
//! for result in prolly.range(&tree, b"b", Some(b"c")).unwrap() {
//!     let (key, val) = result.unwrap();
//!     // Only yields "b" -> "2"
//! }
//! ```
//!
//! ## Diff and Merge
//!
//! ```rust
//! use prolly::{Prolly, MemStore, Config, Diff};
//!
//! let store = MemStore::new();
//! let prolly = Prolly::new(store, Config::default());
//!
//! // Create base tree
//! let base = prolly.create();
//! let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
//!
//! // Create two divergent branches
//! let left = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
//! let right = prolly.put(&base, b"c".to_vec(), b"3".to_vec()).unwrap();
//!
//! // Compute diff
//! let diffs = prolly.diff(&base, &left).unwrap();
//! // diffs contains: Added { key: b"b", val: b"2" }
//!
//! // Three-way merge (no conflicts since changes are disjoint)
//! let merged = prolly.merge(&base, &left, &right, None).unwrap();
//!
//! // Merged tree has all keys: a, b, c
//! assert!(prolly.get(&merged, b"a").unwrap().is_some());
//! assert!(prolly.get(&merged, b"b").unwrap().is_some());
//! assert!(prolly.get(&merged, b"c").unwrap().is_some());
//! ```
//!
//! ## Batch Building
//!
//! For bulk loading data, use [`BatchBuilder`] for parallel tree construction:
//!
//! ```rust
//! use prolly::{BatchBuilder, MemStore, Config, Prolly};
//! use std::sync::Arc;
//!
//! let store = Arc::new(MemStore::new());
//! let config = Config::default();
//!
//! // Build tree from many entries in parallel
//! let mut builder = BatchBuilder::new(store.clone(), config.clone());
//! for i in 0..1000 {
//!     builder.add(format!("key{:04}", i).into_bytes(), format!("val{}", i).into_bytes());
//! }
//! let tree = builder.build().unwrap();
//!
//! // Use the tree with Prolly
//! let prolly = Prolly::new(store, config);
//! let val = prolly.get(&tree, b"key0042").unwrap();
//! assert!(val.is_some());
//! ```
//!
//! ## Custom Storage Backend
//!
//! Implement the [`Store`] trait for custom storage:
//!
//! ```rust
//! use prolly::{Store, BatchOp};
//!
//! struct MyStore {
//!     // Your storage implementation
//! }
//!
//! impl Store for MyStore {
//!     type Error = std::io::Error;
//!
//!     fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
//!         // Implement get
//!         Ok(None)
//!     }
//!
//!     fn put(&self, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
//!         // Implement put
//!         Ok(())
//!     }
//!
//!     fn delete(&self, key: &[u8]) -> Result<(), Self::Error> {
//!         // Implement delete
//!         Ok(())
//!     }
//!
//!     fn batch(&self, ops: &[BatchOp]) -> Result<(), Self::Error> {
//!         // Implement batch operations
//!         Ok(())
//!     }
//! }
//! ```
//!
//! ## Configuration
//!
//! Customize tree behavior with [`Config`]:
//!
//! ```rust
//! use prolly::{Config, Encoding};
//!
//! let config = Config::builder()
//!     .min_chunk_size(4)        // Min entries before considering split
//!     .max_chunk_size(1024)     // Max entries per node
//!     .chunking_factor(128)     // Controls average node size
//!     .hash_seed(42)            // Seed for boundary detection
//!     .encoding(Encoding::Raw)  // Value encoding type
//!     .build();
//! ```
//!
//! ## Advanced Extensibility
//!
//! The library provides extensible traits for advanced use cases:
//!
//! ### Streaming Diff
//!
//! Use [`StreamingDiffer`] for memory-efficient diff operations on large trees:
//!
//! ```rust
//! use prolly::{Prolly, MemStore, Config, Diff};
//! use std::sync::Arc;
//!
//! let store = Arc::new(MemStore::new());
//! let prolly = Prolly::new(store.clone(), Config::default());
//!
//! let base = prolly.create();
//! let other = prolly.put(&base, b"key".to_vec(), b"val".to_vec()).unwrap();
//!
//! // Stream differences lazily (memory-efficient for large trees)
//! for diff_result in prolly.stream_diff(&base, &other).unwrap() {
//!     match diff_result {
//!         Ok(diff) => println!("{:?}", diff),
//!         Err(e) => eprintln!("Error: {}", e),
//!     }
//! }
//! ```
//!
//! ### CRDT Merge
//!
//! Use [`ConflictFreeMerger`] for automatic conflict resolution:
//!
//! ```rust
//! use prolly::{CrdtConfig, MergeStrategy, DeletePolicy, TimestampedValue};
//!
//! // Last-Writer-Wins strategy
//! let lww_config = CrdtConfig::lww();
//!
//! // Multi-Value strategy (preserves all concurrent values)
//! let mv_config = CrdtConfig::multi_value();
//!
//! // Custom merge function
//! let custom_config = CrdtConfig::custom(|_key, left, right| {
//!     // Your merge logic here
//!     left.to_vec()
//! });
//! ```
//!
//! ### Parallel Processing
//!
//! Use [`ParallelRebalancer`] for multi-threaded batch operations:
//!
//! ```rust
//! use prolly::{ParallelConfig, DefaultParallelRebalancer};
//!
//! // Configure parallel processing
//! let config = ParallelConfig {
//!     max_threads: 4,           // Use 4 threads (0 = auto)
//!     parallelism_threshold: 50, // Parallelize when > 50 items
//! };
//!
//! let rebalancer = DefaultParallelRebalancer::new();
//! ```

/// Prolly tree implementation - all core modules are now in this submodule
pub mod prolly;

// Re-export public API from prolly module
pub use prolly::boundary::{is_boundary, is_boundary_config};
pub use prolly::builder::{BatchBuilder, SortedBatchBuilder};
pub use prolly::cid::Cid;
pub use prolly::config::{Config, ConfigBuilder};
pub use prolly::cursor::{Cursor, CursorIterator, DiffCursor};
pub use prolly::encoding::Encoding;
pub use prolly::error::{Conflict, Diff, Error, Mutation, Resolver};
pub use prolly::node::{Node, NodeBuilder};
pub use prolly::{
    append_batch,
    apply_batch_with_rebuild,
    apply_mutations_deferred,
    apply_mutations_to_leaf,
    apply_mutations_to_leaf_binary_search,
    bottom_up_rebuild,
    bottom_up_rebuild_groups,
    build_internal_level,
    compute_affected_spans,
    filter_mutations_for_range,
    group_mutations_by_leaf,
    group_mutations_by_leaf_cursor,
    prefetch_leaves,
    preprocess_mutations,
    rebuild_from_modified_leaves,
    should_use_deferred_rebalancing,
    // Re-export for testing
    split_into_chunks,
    split_oversized_node,
    BatchWriteCollector,
    BatchWriter,
    BatchWriterConfig,
    // CRDT types for conflict-free merging
    ConflictFreeMerger,
    CrdtConfig,
    CustomMergeFn,
    DefaultConflictFreeMerger,
    // Parallel rebalancer types
    DefaultParallelRebalancer,
    DefaultStreamingDiffer,
    DeferredMutationResult,
    DeletePolicy,
    LeafMutationGroup,
    LeafSpan,
    MergeStrategy,
    MultiValueSet,
    MutationBuffer,
    ParallelConfig,
    ParallelRebalancer,
    Prolly,
    RangeIter,
    RebuildResult,
    StreamingDiffer,
    TimestampExtractor,
    TimestampedValue,
};

pub use prolly::stats::TreeStats;
pub use prolly::store::{BatchOp, MemStore, MemStoreError, Store};
#[cfg(feature = "rocksdb")]
pub use prolly::store::{CompressionType, RocksDBConfig, RocksDBStore, RocksDBStoreError};
#[cfg(feature = "sqlite")]
pub use prolly::store::{SqliteStore, SqliteStoreConfig, SqliteStoreError};
pub use prolly::tree::Tree;

// Re-export constants
pub use prolly::encoding::{
    DEFAULT_CHUNKING_FACTOR, DEFAULT_HASH_SEED, DEFAULT_MAX_CHUNK_SIZE, DEFAULT_MIN_CHUNK_SIZE,
    INIT_LEVEL,
};
