//! Prolly tree implementation - modular, trait-based architecture
//!
//! This module provides the main interface for working with Prolly trees through
//! the [`Prolly<S>`] struct. Prolly trees are content-addressable, probabilistically-balanced
//! search trees that combine the efficiency of B+ trees with deterministic merging capabilities.
//!
//! # Overview
//!
//! A Prolly tree is a persistent, immutable data structure that uses content-based chunking
//! to achieve structural sharing and efficient diffs. Key characteristics include:
//!
//! - **Immutable**: All operations return new trees, enabling safe concurrent access
//! - **Content-addressed**: Nodes are identified by their content hash (CID)
//! - **Deterministic**: Same content always produces the same tree structure
//! - **Efficient diffs**: Structural sharing enables fast comparisons between versions
//!
//! # Module Organization
//!
//! The implementation is organized into focused submodules for maintainability:
//!
//! - [`batch`] - Batch mutation operations for efficient bulk modifications
//! - [`diff`] - Tree diff and merge operations for version control semantics
//! - [`range`] - Range iteration for traversing key-value pairs in order
//! - [`rebalance`] - Tree rebalancing logic (node splitting and merging)
//! - [`utils`] - Shared utility functions used across modules
//! - `traits` - Internal trait definitions for future extensibility
//!
//! # Main Types
//!
//! - [`Prolly<S>`] - The main tree manager, generic over storage backend `S`
//! - [`RangeIter`] - Iterator for range queries over key-value pairs
//! - [`BatchWriteCollector`] - Collector for atomic batch writes
//! - [`LeafMutationGroup`] - Mutations grouped by target leaf for batch processing
//!
//! # Quick Start
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
//! let tree = prolly.put(&tree, b"key".to_vec(), b"value".to_vec()).unwrap();
//!
//! // Retrieve values
//! let value = prolly.get(&tree, b"key").unwrap();
//! assert_eq!(value, Some(b"value".to_vec()));
//!
//! // Delete keys
//! let tree = prolly.delete(&tree, b"key").unwrap();
//! assert!(prolly.get(&tree, b"key").unwrap().is_none());
//! ```
//!
//! # Architecture
//!
//! The `Prolly<S>` struct serves as the main API and delegates to specialized modules:
//!
//! - **Core operations** (get, put, delete) are implemented directly in this module
//! - **Rebalancing** is delegated to the [`rebalance`] module for node splitting/merging
//! - **Batch operations** are delegated to the [`batch`] module for atomic bulk updates
//! - **Range iteration** is delegated to the [`range`] module for ordered traversal
//! - **Diff/merge operations** are delegated to the [`diff`] module for version control
//!
//! This separation of concerns enables:
//! - Independent testing of each component
//! - Clear boundaries between responsibilities
//! - Future extensibility through trait-based interfaces
//!
//! # Storage Backend
//!
//! The tree is generic over a storage backend `S` that implements the [`Store`](crate::store::Store)
//! trait. This allows plugging in different storage implementations:
//!
//! - [`MemStore`](crate::store::MemStore) - In-memory storage for testing
//! - Custom implementations for persistent storage (databases, file systems, etc.)
//!
//! # Thread Safety
//!
//! The `Prolly<S>` struct is `Send` and `Sync` when the underlying store is. The immutable
//! nature of trees means multiple threads can safely read from the same tree simultaneously.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

// Core modules - moved from root level
pub mod boundary;
pub mod builder;
pub mod cid;
pub mod config;
pub mod cursor;
pub mod encoding;
pub mod error;
pub mod node;
pub mod stats;
pub mod store;
pub mod tree;

// Public submodules - each handles a specific concern
pub mod batch;
pub mod crdt;
pub mod diff;
pub mod parallel;
pub mod range;
pub mod rebalance;
pub mod streaming;
pub mod utils;

// Internal traits for future extensibility (not exposed publicly)
mod traits;

use cid::Cid;
use config::Config;
use encoding::INIT_LEVEL;
use error::Diff;
use error::Error;
use error::Mutation;
use error::Resolver;
use node::Node;
use stats::TreeStats;
use store::Store;
use tree::Tree;

// Re-export RangeIter for range queries
pub use range::RangeIter;

// Re-export batch types and functions for public API
pub use batch::append_batch;
pub use batch::apply_batch_with_rebuild;
pub use batch::apply_mutations_deferred;
pub use batch::apply_mutations_to_leaf;
pub use batch::apply_mutations_to_leaf_binary_search;
pub use batch::bottom_up_rebuild;
pub use batch::bottom_up_rebuild_groups;
pub use batch::build_internal_level;
pub use batch::compute_affected_spans;
pub use batch::filter_mutations_for_range;
pub use batch::get_max_key;
pub use batch::group_mutations_by_leaf;
pub use batch::group_mutations_by_leaf_cursor;
pub use batch::prefetch_leaves;
pub use batch::preprocess_mutations;
pub use batch::rebuild_from_modified_leaves;
pub use batch::should_use_deferred_rebalancing;
pub use batch::split_oversized_node;
pub use batch::BatchWriteCollector;
pub use batch::BatchWriter;
pub use batch::BatchWriterConfig;
pub use batch::DeferredMutationResult;
pub use batch::LeafMutationGroup;
pub use batch::LeafSpan;
pub use batch::MutationBuffer;
pub use batch::RebuildResult;

// Re-export rebalance functions for testing
pub use rebalance::split_into_chunks;

// Re-export streaming diff types
pub use streaming::{DefaultStreamingDiffer, StreamingDiffer};

// Re-export CRDT types for conflict-free merging
pub use crdt::{
    ConflictFreeMerger, CrdtConfig, CustomMergeFn, DefaultConflictFreeMerger, DeletePolicy,
    MergeStrategy, MultiValueSet, TimestampExtractor, TimestampedValue,
};

// Re-export parallel rebalancer types
pub use parallel::{DefaultParallelRebalancer, ParallelConfig, ParallelRebalancer};

/// Prolly tree manager
///
/// Provides the high-level API for working with Prolly trees.
/// Generic over the storage backend `S`.
///
/// # Example
/// ```
/// use prolly::{Prolly, MemStore, Config};
///
/// let store = MemStore::new();
/// let prolly = Prolly::new(store, Config::default());
/// let tree = prolly.create();
/// ```
pub struct Prolly<S: Store> {
    store: S,
    config: Config,
    node_cache: RwLock<HashMap<Cid, Arc<Node>>>,
    rightmost_path_cache: RwLock<Option<(Cid, Vec<CachedRightmostPathEntry>)>>,
}

#[derive(Clone)]
pub(crate) struct CachedRightmostPathEntry {
    pub cid: Cid,
    pub node: Node,
    pub child_index: usize,
}

fn is_rightmost_append_path(path: &[(Node, usize)], key: &[u8]) -> bool {
    let Some((leaf, _)) = path.last() else {
        return true;
    };

    if !leaf.leaf || leaf.search(key) != Err(leaf.len()) {
        return false;
    }

    path.iter()
        .all(|(node, child_index)| *child_index + 1 == node.len())
}

impl<S: Store> Prolly<S> {
    /// Create a new Prolly tree manager with the given store and configuration.
    ///
    /// # Arguments
    /// * `store` - Storage backend implementing the `Store` trait
    /// * `config` - Tree configuration (chunking parameters, encoding, etc.)
    pub fn new(store: S, config: Config) -> Self {
        Self {
            store,
            config,
            node_cache: RwLock::new(HashMap::new()),
            rightmost_path_cache: RwLock::new(None),
        }
    }

    /// Create a new empty tree.
    ///
    /// Returns a `Tree` with no root (empty tree).
    pub fn create(&self) -> Tree {
        Tree {
            root: None,
            config: self.config.clone(),
        }
    }

    /// Get value by key from the tree.
    ///
    /// Traverses from root to leaf using binary search at each level.
    ///
    /// # Arguments
    /// * `tree` - The tree to search
    /// * `key` - The key to look up
    ///
    /// # Returns
    /// * `Ok(Some(value))` if the key exists
    /// * `Ok(None)` if the key does not exist
    /// * `Err` on storage or deserialization errors
    pub fn get(&self, tree: &Tree, key: &[u8]) -> Result<Option<Vec<u8>>, Error> {
        let Some(root_cid) = &tree.root else {
            return Ok(None);
        };

        let mut cid = root_cid.clone();
        loop {
            let node = self.load_arc(&cid)?;
            let idx = match node.search(key) {
                Ok(i) => i,
                Err(i) => {
                    if i == 0 {
                        return Ok(None);
                    } else {
                        i - 1
                    }
                }
            };

            if node.leaf {
                if node.keys.get(idx).map(|k| k.as_slice()) == Some(key) {
                    return Ok(Some(node.vals[idx].clone()));
                }
                return Ok(None);
            }

            // Descend to child
            cid = Cid(node.vals[idx]
                .as_slice()
                .try_into()
                .map_err(|_| Error::InvalidNode)?);
        }
    }

    /// Insert or update a key-value pair in the tree.
    ///
    /// This operation is immutable - it returns a new tree rather than
    /// modifying the existing one.
    ///
    /// # Arguments
    /// * `tree` - The tree to modify
    /// * `key` - The key to insert/update
    /// * `val` - The value to associate with the key
    ///
    /// # Returns
    /// * `Ok(new_tree)` with the updated tree
    /// * `Err` on storage or deserialization errors
    ///
    /// # Idempotence
    /// If the key already exists with the same value, returns the original tree unchanged.
    pub fn put(&self, tree: &Tree, key: Vec<u8>, val: Vec<u8>) -> Result<Tree, Error> {
        // Build path to leaf
        let path = self.find_path(tree, &key)?;

        if is_rightmost_append_path(&path, &key) {
            return batch::append_upsert_with_path(self, tree, key, val, &path);
        }

        // Insert into leaf
        let mut node = path
            .last()
            .map(|(n, _)| n.clone())
            .unwrap_or_else(|| self.new_leaf_node());

        match node.search(&key) {
            Ok(i) => {
                if node.vals[i] == val {
                    return Ok(tree.clone()); // No change (idempotent)
                }
                node.vals[i] = val;
            }
            Err(i) => {
                node.keys.insert(i, key);
                node.vals.insert(i, val);
            }
        }

        // Rebalance and persist the O(height) changed path atomically. This
        // keeps random updates localized while avoiding one disk transaction
        // per rewritten node on durable stores.
        let mut collector = batch::BatchWriteCollector::new_cached();
        let new_root = rebalance::rebalance_with_collector(
            self,
            node,
            &path[..path.len().saturating_sub(1)],
            &mut collector,
        )?
        .ok_or(Error::InvalidNode)?;
        collector.flush(self.store())?;
        collector.cache_nodes(self)?;

        Ok(Tree {
            root: Some(new_root),
            config: tree.config.clone(),
        })
    }

    /// Delete a key from the tree.
    ///
    /// This operation is immutable - it returns a new tree rather than
    /// modifying the existing one.
    ///
    /// # Arguments
    /// * `tree` - The tree to modify
    /// * `key` - The key to delete
    ///
    /// # Returns
    /// * `Ok(new_tree)` with the key removed (or unchanged if key didn't exist)
    /// * `Err` on storage or deserialization errors
    ///
    /// # Idempotence
    /// If the key doesn't exist, returns the original tree unchanged.
    pub fn delete(&self, tree: &Tree, key: &[u8]) -> Result<Tree, Error> {
        let Some(_) = &tree.root else {
            return Ok(tree.clone());
        };

        let path = self.find_path(tree, key)?;
        let Some((mut node, _)) = path.last().cloned() else {
            return Ok(tree.clone());
        };

        // Remove from leaf
        if let Ok(i) = node.search(key) {
            node.keys.remove(i);
            node.vals.remove(i);
        } else {
            return Ok(tree.clone()); // Key not found (idempotent)
        }

        // Handle empty tree
        if node.is_empty() && path.len() == 1 {
            return Ok(Tree {
                root: None,
                config: tree.config.clone(),
            });
        }

        // Rebalance and persist the O(height) changed path atomically, matching
        // the localized write behavior used by put.
        let mut collector = batch::BatchWriteCollector::new_cached();
        let new_root = rebalance::rebalance_with_collector(
            self,
            node,
            &path[..path.len().saturating_sub(1)],
            &mut collector,
        )?;
        collector.flush(self.store())?;
        collector.cache_nodes(self)?;

        Ok(Tree {
            root: new_root,
            config: tree.config.clone(),
        })
    }

    /// Iterate over a range of key-value pairs.
    ///
    /// Returns an iterator that yields `(key, value)` pairs in lexicographic order,
    /// starting from `start` (inclusive) up to `end` (exclusive). Supports both
    /// inclusive start bounds and exclusive end bounds.
    ///
    /// # Arguments
    /// * `tree` - The tree to iterate over
    /// * `start` - The starting key (inclusive). Use `&[]` to start from the beginning.
    /// * `end` - Optional ending key (exclusive). Use `None` to iterate to the end.
    ///
    /// # Returns
    /// * `Ok(RangeIter)` - An iterator over the range
    /// * `Err` on storage or deserialization errors during path finding
    ///
    /// # Example
    /// ```
    /// use prolly::{Prolly, MemStore, Config};
    ///
    /// let store = MemStore::new();
    /// let prolly = Prolly::new(store, Config::default());
    /// let tree = prolly.create();
    ///
    /// // Insert some data
    /// let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
    /// let tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();
    /// let tree = prolly.put(&tree, b"c".to_vec(), b"3".to_vec()).unwrap();
    ///
    /// // Iterate over all keys
    /// for result in prolly.range(&tree, &[], None).unwrap() {
    ///     let (key, val) = result.unwrap();
    ///     println!("{:?} -> {:?}", key, val);
    /// }
    ///
    /// // Iterate over a specific range [b, c)
    /// for result in prolly.range(&tree, b"b", Some(b"c")).unwrap() {
    ///     let (key, val) = result.unwrap();
    ///     println!("{:?} -> {:?}", key, val);
    /// }
    /// ```
    pub fn range<'a>(
        &'a self,
        tree: &Tree,
        start: &[u8],
        end: Option<&[u8]>,
    ) -> Result<RangeIter<'a, S>, Error> {
        range::create_range_iter(self, tree, start, end)
    }

    /// Compute the difference between two trees.
    ///
    /// Returns a vector of `Diff` entries representing the changes needed to
    /// transform `base` into `other`. Yields Added entries for keys that exist in
    /// other but not in base, Changed entries for keys with different values, and
    /// Removed entries for keys that exist in base but not in other.
    ///
    /// # Arguments
    /// * `base` - The base tree to compare from
    /// * `other` - The other tree to compare to
    ///
    /// # Returns
    /// * `Ok(Vec<Diff>)` - A vector of differences
    /// * `Err` on storage or deserialization errors
    ///
    /// # Short-circuit
    /// If both trees have the same root CID, returns an empty vector immediately.
    ///
    /// # Example
    /// ```
    /// use prolly::{Prolly, MemStore, Config};
    ///
    /// let store = MemStore::new();
    /// let prolly = Prolly::new(store, Config::default());
    /// let base = prolly.create();
    ///
    /// let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
    /// let other = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
    ///
    /// let diffs = prolly.diff(&base, &other).unwrap();
    /// // diffs contains Added { key: b"b", val: b"2" }
    /// ```
    pub fn diff(&self, base: &Tree, other: &Tree) -> Result<Vec<Diff>, Error> {
        diff::compute_diff(self, base, other)
    }

    /// Compute the difference between two trees within a half-open key range.
    ///
    /// Returns only changes whose key is in `[start, end)`. Unlike collecting
    /// two ranges and comparing them, this walks the tree shape directly and
    /// skips equal or out-of-range subtrees by CID and key span.
    pub fn range_diff(
        &self,
        base: &Tree,
        other: &Tree,
        start: &[u8],
        end: Option<&[u8]>,
    ) -> Result<Vec<Diff>, Error> {
        diff::compute_range_diff(self, base, other, start, end)
    }

    /// Merge two trees using three-way merge.
    ///
    /// Performs a three-way merge using `base` as the common ancestor.
    /// Changes from both `left` and `right` are combined into a single tree.
    /// Uses the diff algorithm to efficiently identify entries to add.
    ///
    /// # Arguments
    /// * `base` - The common ancestor tree
    /// * `left` - The left branch tree
    /// * `right` - The right branch tree
    /// * `resolver` - Optional conflict resolver function
    ///
    /// # Returns
    /// * `Ok(merged_tree)` - The merged tree
    /// * `Err(Error::Conflict)` - If a conflict occurs and no resolver is provided
    ///   or the resolver returns `None`
    ///
    /// # Conflict Handling
    /// A conflict occurs when both `left` and `right` modify the same key differently
    /// from `base`. When this happens:
    /// - If a resolver is provided, it's called with the conflict information
    /// - If the resolver returns `Some(value)`, that value is used
    /// - If the resolver returns `None` or no resolver is provided, an error is returned
    ///
    /// Keys that have the same value in both trees are included once in the result.
    ///
    /// # Example
    /// ```
    /// use prolly::{Prolly, MemStore, Config, Resolver};
    ///
    /// let store = MemStore::new();
    /// let prolly = Prolly::new(store, Config::default());
    /// let base = prolly.create();
    ///
    /// // Create base tree
    /// let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
    ///
    /// // Create divergent branches
    /// let left = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
    /// let right = prolly.put(&base, b"c".to_vec(), b"3".to_vec()).unwrap();
    ///
    /// // Merge without conflicts
    /// let merged = prolly.merge(&base, &left, &right, None).unwrap();
    ///
    /// // Merged tree has all keys
    /// assert!(prolly.get(&merged, b"a").unwrap().is_some());
    /// assert!(prolly.get(&merged, b"b").unwrap().is_some());
    /// assert!(prolly.get(&merged, b"c").unwrap().is_some());
    /// ```
    pub fn merge(
        &self,
        base: &Tree,
        left: &Tree,
        right: &Tree,
        resolver: Option<Resolver>,
    ) -> Result<Tree, Error> {
        diff::merge_trees(self, base, left, right, resolver)
    }

    /// Collect comprehensive statistics about a tree
    ///
    /// Traverses the entire tree once, gathering metrics about structure,
    /// size, distribution, and efficiency.
    ///
    /// # Arguments
    /// * `tree` - The tree to analyze
    ///
    /// # Returns
    /// * `Ok(TreeStats)` - Collected statistics
    /// * `Err(Error)` - On storage or deserialization errors
    ///
    /// # Example
    /// ```
    /// use prolly::{Prolly, MemStore, Config};
    ///
    /// let store = MemStore::new();
    /// let prolly = Prolly::new(store, Config::default());
    /// let tree = prolly.create();
    ///
    /// let tree = prolly.put(&tree, b"key".to_vec(), b"value".to_vec()).unwrap();
    /// let stats = prolly.collect_stats(&tree).unwrap();
    /// println!("{:?}", stats);
    /// ```
    pub fn collect_stats(&self, tree: &Tree) -> Result<TreeStats, Error> {
        // Handle empty tree case
        let Some(root_cid) = &tree.root else {
            let mut stats = TreeStats::new();
            stats.finalize();
            return Ok(stats);
        };

        // Initialize TreeStats
        let mut stats = TreeStats::new();

        // Call recursive helper
        self.collect_stats_recursive(root_cid, &mut stats)?;

        // Finalize statistics
        stats.finalize();

        // Return result
        Ok(stats)
    }

    /// Recursive helper for collecting statistics
    ///
    /// Traverses the tree depth-first, accumulating statistics at each node.
    ///
    /// # Arguments
    /// * `cid` - The CID of the current node
    /// * `stats` - Mutable reference to the statistics accumulator
    ///
    /// # Returns
    /// * `Ok(())` - Statistics accumulated successfully
    /// * `Err(Error)` - On storage or deserialization errors
    fn collect_stats_recursive(&self, cid: &Cid, stats: &mut TreeStats) -> Result<(), Error> {
        // Load node from store (propagate errors)
        let node = self.load(cid)?;

        // Call stats.accumulate(&node)
        stats.accumulate(&node);

        // For internal nodes, recursively visit children
        if !node.leaf {
            for child_cid_bytes in &node.vals {
                // Handle CID conversion errors
                let child_cid = Cid(child_cid_bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| Error::InvalidNode)?);
                self.collect_stats_recursive(&child_cid, stats)?;
            }
        }

        Ok(())
    }

    /// Create a cursor positioned at the given key.
    ///
    /// Returns a cursor that can be used for efficient traversal through the tree.
    /// The cursor is positioned at the key if it exists, or at the greatest key
    /// less than or equal to the target key.
    ///
    /// # Arguments
    /// * `tree` - The tree to navigate
    /// * `key` - The key to position at
    ///
    /// # Returns
    /// * `Ok(Cursor)` - A cursor positioned at or near the key
    /// * `Err` - If a storage error occurs
    ///
    /// # Example
    /// ```
    /// use prolly::{Prolly, MemStore, Config};
    ///
    /// let store = std::sync::Arc::new(MemStore::new());
    /// let prolly = Prolly::new(store.clone(), Config::default());
    /// let mut tree = prolly.create();
    ///
    /// tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
    /// tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();
    ///
    /// let cursor = prolly.cursor(&tree, b"a").unwrap();
    /// assert!(cursor.is_valid());
    /// assert_eq!(cursor.get_key(), Some(b"a".as_slice()));
    /// ```
    pub fn cursor(&self, tree: &Tree, key: &[u8]) -> Result<cursor::Cursor, Error> {
        cursor::Cursor::at_item(&self.store, tree, key)
    }

    /// Create a cursor iterator for range queries.
    ///
    /// Returns an iterator that yields (key, value) pairs in lexicographic order,
    /// starting from `start` (inclusive) up to `end` (exclusive).
    ///
    /// # Arguments
    /// * `tree` - The tree to iterate over
    /// * `start` - The starting key (inclusive). Use `&[]` to start from the beginning.
    /// * `end` - Optional ending key (exclusive). Use `None` to iterate to the end.
    ///
    /// # Returns
    /// * `Ok(CursorIterator)` - An iterator over the range
    /// * `Err` - If a storage error occurs during cursor creation
    ///
    /// # Example
    /// ```
    /// use prolly::{Prolly, MemStore, Config};
    ///
    /// let store = std::sync::Arc::new(MemStore::new());
    /// let prolly = Prolly::new(store.clone(), Config::default());
    /// let mut tree = prolly.create();
    ///
    /// tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
    /// tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();
    /// tree = prolly.put(&tree, b"c".to_vec(), b"3".to_vec()).unwrap();
    ///
    /// // Iterate over range [a, c)
    /// let iter = prolly.range_cursor(&tree, b"a", Some(b"c")).unwrap();
    /// let entries: Vec<_> = iter.collect();
    /// assert_eq!(entries.len(), 2); // "a" and "b"
    /// ```
    pub fn range_cursor<'a>(
        &'a self,
        tree: &Tree,
        start: &[u8],
        end: Option<&[u8]>,
    ) -> Result<cursor::CursorIterator<'a, S>, Error> {
        let cursor = cursor::Cursor::at_item(&self.store, tree, start)?;
        Ok(cursor::CursorIterator::with_bounds(
            cursor,
            &self.store,
            Some(start.to_vec()),
            end.map(|e| e.to_vec()),
        ))
    }

    /// Create a streaming diff iterator between two trees.
    ///
    /// Returns an iterator that yields `Diff` entries representing the changes
    /// needed to transform `base` into `other`. More memory-efficient than
    /// `diff()` for large trees as it doesn't collect all differences upfront.
    ///
    /// # Arguments
    /// * `base` - The base tree to compare from
    /// * `other` - The other tree to compare to
    ///
    /// # Returns
    /// * `Ok(DiffCursor)` - A streaming diff iterator
    /// * `Err(Error)` - If cursor initialization fails
    ///
    /// # Example
    /// ```rust
    /// use prolly::{Prolly, MemStore, Config, Diff};
    /// use std::sync::Arc;
    ///
    /// let store = Arc::new(MemStore::new());
    /// let prolly = Prolly::new(store.clone(), Config::default());
    ///
    /// let base = prolly.create();
    /// let other = prolly.put(&base, b"key".to_vec(), b"val".to_vec()).unwrap();
    ///
    /// // Stream differences
    /// for diff in prolly.diff_cursor(&base, &other).unwrap() {
    ///     println!("{:?}", diff);
    /// }
    ///
    /// // Or collect into Vec (equivalent to diff())
    /// let diffs: Vec<Diff> = prolly.diff_cursor(&base, &other).unwrap().collect();
    /// ```
    pub fn diff_cursor<'a>(
        &'a self,
        base: &Tree,
        other: &Tree,
    ) -> Result<cursor::DiffCursor<'a, S>, Error> {
        cursor::DiffCursor::new(&self.store, base, other)
    }

    /// Create a streaming diff iterator between two trees.
    ///
    /// Returns an iterator that yields `Result<Diff, Error>` entries representing
    /// the changes needed to transform `base` into `other`. This method walks
    /// the same content-addressed structure as eager diff, so equal subtrees are
    /// skipped by CID and only changed subtrees are visited.
    ///
    /// # Arguments
    /// * `base` - The base tree to compare from
    /// * `other` - The other tree to compare to
    ///
    /// # Returns
    /// * `Ok(impl Iterator)` - A streaming diff iterator yielding `Result<Diff, Error>`
    /// * `Err(Error)` - If cursor initialization fails
    ///
    /// # Short-circuit
    /// If both trees have the same root CID, returns an empty iterator immediately.
    ///
    /// # Example
    /// ```rust
    /// use prolly::{Prolly, MemStore, Config, Diff};
    /// use std::sync::Arc;
    ///
    /// let store = Arc::new(MemStore::new());
    /// let prolly = Prolly::new(store.clone(), Config::default());
    ///
    /// let base = prolly.create();
    /// let other = prolly.put(&base, b"key".to_vec(), b"val".to_vec()).unwrap();
    ///
    /// // Stream differences with error handling
    /// for diff_result in prolly.stream_diff(&base, &other).unwrap() {
    ///     match diff_result {
    ///         Ok(diff) => println!("{:?}", diff),
    ///         Err(e) => eprintln!("Error: {}", e),
    ///     }
    /// }
    ///
    /// // Collect successful diffs
    /// let diffs: Vec<Diff> = prolly.stream_diff(&base, &other)
    ///     .unwrap()
    ///     .filter_map(|r| r.ok())
    ///     .collect();
    /// ```
    pub fn stream_diff<'a>(
        &'a self,
        base: &Tree,
        other: &Tree,
    ) -> Result<Box<dyn Iterator<Item = Result<Diff, Error>> + 'a>, Error> {
        Ok(Box::new(diff::stream_diff(self, base, other)))
    }

    /// Load a node by its CID from the store.
    pub(crate) fn load(&self, cid: &Cid) -> Result<Node, Error> {
        Ok(self.load_arc(cid)?.as_ref().clone())
    }

    /// Load a node by its CID, reusing the in-process immutable node cache.
    pub(crate) fn load_arc(&self, cid: &Cid) -> Result<Arc<Node>, Error> {
        if let Ok(cache) = self.node_cache.read() {
            if let Some(node) = cache.get(cid) {
                return Ok(node.clone());
            }
        }

        let bytes = self
            .store
            .get(cid.as_bytes())
            .map_err(|e| Error::Store(Box::new(e)))?
            .ok_or_else(|| Error::NotFound(cid.clone()))?;
        let node = Arc::new(Node::from_bytes(&bytes)?);

        if let Ok(mut cache) = self.node_cache.write() {
            let cached = cache.entry(cid.clone()).or_insert_with(|| node.clone());
            return Ok(cached.clone());
        }

        Ok(node)
    }

    /// Load nodes by CID in input order, batching cache misses through the store.
    pub(crate) fn load_many_ordered(&self, cids: &[Cid]) -> Result<Vec<Arc<Node>>, Error> {
        let mut nodes = vec![None; cids.len()];
        let mut missing_positions: HashMap<Cid, Vec<usize>> = HashMap::new();
        let mut missing_cids = Vec::new();

        if let Ok(cache) = self.node_cache.read() {
            for (idx, cid) in cids.iter().enumerate() {
                if let Some(node) = cache.get(cid) {
                    nodes[idx] = Some(node.clone());
                } else {
                    if !missing_positions.contains_key(cid) {
                        missing_cids.push(cid.clone());
                        missing_positions.insert(cid.clone(), Vec::new());
                    }
                    missing_positions
                        .get_mut(cid)
                        .ok_or(Error::InvalidNode)?
                        .push(idx);
                }
            }
        } else {
            for (idx, cid) in cids.iter().enumerate() {
                if !missing_positions.contains_key(cid) {
                    missing_cids.push(cid.clone());
                    missing_positions.insert(cid.clone(), Vec::new());
                }
                missing_positions
                    .get_mut(cid)
                    .ok_or(Error::InvalidNode)?
                    .push(idx);
            }
        }

        if !missing_cids.is_empty() {
            let keys = missing_cids
                .iter()
                .map(|cid| cid.as_bytes())
                .collect::<Vec<_>>();
            let loaded = self
                .store
                .batch_get_ordered(&keys)
                .map_err(|e| Error::Store(Box::new(e)))?;

            if loaded.len() != missing_cids.len() {
                return Err(Error::InvalidNode);
            }

            let mut cache = self.node_cache.write().ok();
            for (cid, bytes) in missing_cids.into_iter().zip(loaded) {
                let bytes = bytes.ok_or_else(|| Error::NotFound(cid.clone()))?;
                let node = Arc::new(Node::from_bytes(&bytes)?);
                let node = if let Some(cache) = cache.as_mut() {
                    cache
                        .entry(cid.clone())
                        .or_insert_with(|| node.clone())
                        .clone()
                } else {
                    node
                };
                let positions = missing_positions.remove(&cid).ok_or(Error::InvalidNode)?;
                for idx in positions {
                    nodes[idx] = Some(node.clone());
                }
            }
        }

        nodes
            .into_iter()
            .collect::<Option<Vec<_>>>()
            .ok_or(Error::InvalidNode)
    }

    /// Save a node to the store and return its CID.
    pub(crate) fn save(&self, node: &Node) -> Result<Cid, Error> {
        let bytes = node.to_bytes();
        let cid = Cid::from_bytes(&bytes);
        self.store
            .put(cid.as_bytes(), &bytes)
            .map_err(|e| Error::Store(Box::new(e)))?;
        if let Ok(mut cache) = self.node_cache.write() {
            cache.insert(cid.clone(), Arc::new(node.clone()));
        }
        Ok(cid)
    }

    pub(crate) fn cache_node(&self, cid: Cid, node: Node) {
        if let Ok(mut cache) = self.node_cache.write() {
            cache.insert(cid, Arc::new(node));
        }
    }

    pub(crate) fn cached_rightmost_path(
        &self,
        root: &Cid,
    ) -> Option<Vec<CachedRightmostPathEntry>> {
        self.rightmost_path_cache
            .read()
            .ok()
            .and_then(|cached| match cached.as_ref() {
                Some((cached_root, path)) if cached_root == root => Some(path.clone()),
                _ => None,
            })
    }

    pub(crate) fn cache_rightmost_path(&self, root: Cid, path: Vec<CachedRightmostPathEntry>) {
        if let Ok(mut cache) = self.rightmost_path_cache.write() {
            *cache = Some((root, path));
        }
    }

    /// Clear the in-process immutable node cache.
    ///
    /// This is mostly useful after external store maintenance or tests that
    /// intentionally mutate the backing store outside the Prolly API.
    pub fn clear_cache(&self) {
        if let Ok(mut cache) = self.node_cache.write() {
            cache.clear();
        }
        if let Ok(mut cache) = self.rightmost_path_cache.write() {
            *cache = None;
        }
    }

    /// Return the number of cached nodes in this Prolly manager.
    pub fn cache_len(&self) -> usize {
        self.node_cache.read().map(|cache| cache.len()).unwrap_or(0)
    }

    /// Get a reference to the underlying store.
    pub(crate) fn store(&self) -> &S {
        &self.store
    }

    /// Find the path from root to the key.
    ///
    /// Returns a vector of (node, index) pairs representing the traversal path.
    /// The last element is the leaf node where the key would be found/inserted.
    pub(crate) fn find_path(&self, tree: &Tree, key: &[u8]) -> Result<Vec<(Node, usize)>, Error> {
        let mut path = Vec::new();

        let Some(root_cid) = &tree.root else {
            return Ok(path);
        };

        let mut cid = root_cid.clone();
        loop {
            let node = self.load(&cid)?;
            let idx = match node.search(key) {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            };

            let is_leaf = node.leaf;
            path.push((node.clone(), idx));

            if is_leaf {
                break;
            }

            cid = Cid(node.vals[idx]
                .as_slice()
                .try_into()
                .map_err(|_| Error::InvalidNode)?);
        }

        Ok(path)
    }

    /// Create a new leaf node with config settings.
    pub(crate) fn new_leaf_node(&self) -> Node {
        Node::builder()
            .leaf(true)
            .level(INIT_LEVEL)
            .min_chunk_size(self.config.min_chunk_size)
            .max_chunk_size(self.config.max_chunk_size)
            .chunking_factor(self.config.chunking_factor)
            .hash_seed(self.config.hash_seed)
            .encoding(self.config.encoding.clone())
            .build()
    }

    /// Create a new internal node with config settings.
    pub(crate) fn new_internal_node(&self, level: u8) -> Node {
        Node::builder()
            .leaf(false)
            .level(level)
            .min_chunk_size(self.config.min_chunk_size)
            .max_chunk_size(self.config.max_chunk_size)
            .chunking_factor(self.config.chunking_factor)
            .hash_seed(self.config.hash_seed)
            .encoding(self.config.encoding.clone())
            .build()
    }

    /// Create a new node with the same settings as an existing node.
    pub(crate) fn new_node_like(&self, template: &Node) -> Node {
        Node::builder()
            .leaf(template.leaf)
            .level(template.level)
            .min_chunk_size(template.min_chunk_size)
            .max_chunk_size(template.max_chunk_size)
            .chunking_factor(template.chunking_factor)
            .hash_seed(template.hash_seed)
            .encoding(template.encoding.clone())
            .build()
    }

    /// Apply multiple mutations to a tree in a single optimized operation.
    ///
    /// This method enables efficient bulk modifications (upserts and deletes) to an
    /// existing tree. Mutations are sorted by key, deduplicated (last-write-wins),
    /// grouped by affected leaf, and applied with a single atomic batch write.
    ///
    /// # Arguments
    /// * `tree` - The tree to modify
    /// * `mutations` - Vector of mutations to apply
    ///
    /// # Returns
    /// * `Ok(Tree)` - New tree with all mutations applied
    /// * `Err(Error)` - On storage or processing errors
    ///
    /// # Behavior
    /// - Mutations are sorted by key for efficient processing
    /// - Duplicate keys use last-write-wins semantics
    /// - All new nodes are written atomically via Store::batch
    /// - The input tree is not modified (immutable operation)
    ///
    /// # Example
    /// ```
    /// use prolly::{Prolly, MemStore, Config, Mutation};
    ///
    /// let store = MemStore::new();
    /// let prolly = Prolly::new(store, Config::default());
    /// let tree = prolly.create();
    ///
    /// let mutations = vec![
    ///     Mutation::Upsert { key: b"a".to_vec(), val: b"1".to_vec() },
    ///     Mutation::Upsert { key: b"b".to_vec(), val: b"2".to_vec() },
    ///     Mutation::Delete { key: b"c".to_vec() },
    /// ];
    ///
    /// let new_tree = prolly.batch(&tree, mutations).unwrap();
    /// ```
    pub fn batch(&self, tree: &Tree, mutations: Vec<Mutation>) -> Result<Tree, Error> {
        batch::apply_batch(self, tree, mutations)
    }

    /// Merge two trees using CRDT semantics for automatic conflict resolution.
    ///
    /// Unlike the standard `merge()` method which can return `Error::Conflict`,
    /// this method uses CRDT (Conflict-free Replicated Data Type) semantics to
    /// automatically resolve all conflicts. This makes it suitable for distributed
    /// systems where concurrent modifications are common.
    ///
    /// # Arguments
    /// * `base` - The common ancestor tree
    /// * `left` - The left branch tree
    /// * `right` - The right branch tree
    /// * `config` - CRDT configuration specifying merge strategy and policies
    ///
    /// # Returns
    /// * `Ok(Tree)` - The merged tree (never returns `Error::Conflict`)
    /// * `Err(Error)` - Only on storage or deserialization errors
    ///
    /// # Merge Strategies
    /// - **LastWriterWins (LWW)**: Value with higher timestamp wins
    /// - **MultiValue (MV)**: Preserve all concurrent values as a set
    /// - **Custom**: User-provided merge function
    ///
    /// # Example
    /// ```rust
    /// use prolly::{Prolly, MemStore, Config, CrdtConfig, MergeStrategy};
    /// use std::sync::Arc;
    ///
    /// let store = Arc::new(MemStore::new());
    /// let prolly = Prolly::new(store.clone(), Config::default());
    ///
    /// let base = prolly.create();
    /// let base = prolly.put(&base, b"key".to_vec(), b"value".to_vec()).unwrap();
    ///
    /// // Create divergent branches
    /// let left = prolly.put(&base, b"key".to_vec(), b"left".to_vec()).unwrap();
    /// let right = prolly.put(&base, b"key".to_vec(), b"right".to_vec()).unwrap();
    ///
    /// // CRDT merge - never fails with conflict
    /// let config = CrdtConfig::default();
    /// let merged = prolly.crdt_merge(&base, &left, &right, &config).unwrap();
    /// ```
    pub fn crdt_merge(
        &self,
        base: &Tree,
        left: &Tree,
        right: &Tree,
        config: &crdt::CrdtConfig,
    ) -> Result<Tree, Error> {
        let merger = crdt::DefaultConflictFreeMerger::new();
        crdt::ConflictFreeMerger::crdt_merge(&merger, &self.store, base, left, right, config)
    }

    /// Apply batch mutations with parallel processing.
    ///
    /// This method enables efficient bulk modifications using parallel processing
    /// for large trees. It groups mutations by target leaf and processes independent
    /// leaf groups in parallel when beneficial.
    ///
    /// # Arguments
    /// * `tree` - The tree to modify
    /// * `mutations` - Vector of mutations to apply
    /// * `config` - Parallel configuration controlling thread count and threshold
    ///
    /// # Returns
    /// * `Ok(Tree)` - New tree with all mutations applied
    /// * `Err(Error)` - On storage or processing errors
    ///
    /// # Behavior
    /// - Falls back to sequential processing when below the parallelism threshold
    /// - Uses batch_put for efficient I/O
    /// - Maintains all tree invariants
    ///
    /// # Example
    /// ```rust
    /// use prolly::{Prolly, MemStore, Config, Mutation, ParallelConfig};
    /// use std::sync::Arc;
    ///
    /// let store = Arc::new(MemStore::new());
    /// let prolly = Prolly::new(store.clone(), Config::default());
    /// let tree = prolly.create();
    ///
    /// let mutations = vec![
    ///     Mutation::Upsert { key: b"a".to_vec(), val: b"1".to_vec() },
    ///     Mutation::Upsert { key: b"b".to_vec(), val: b"2".to_vec() },
    /// ];
    ///
    /// let config = ParallelConfig::default();
    /// let new_tree = prolly.parallel_batch(&tree, mutations, &config).unwrap();
    /// ```
    pub fn parallel_batch(
        &self,
        tree: &Tree,
        mutations: Vec<Mutation>,
        config: &parallel::ParallelConfig,
    ) -> Result<Tree, Error> {
        let rebalancer = parallel::DefaultParallelRebalancer::new();
        parallel::ParallelRebalancer::parallel_batch(
            &rebalancer,
            &self.store,
            self,
            tree,
            mutations,
            config,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use error::Diff;
    use std::collections::BTreeMap;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use store::{BatchOp, MemStore};

    #[derive(Debug)]
    struct CountingStoreError;

    impl std::fmt::Display for CountingStoreError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("counting store error")
        }
    }

    impl std::error::Error for CountingStoreError {}

    #[derive(Default)]
    struct CountingStore {
        data: Mutex<BTreeMap<Vec<u8>, Vec<u8>>>,
        put_calls: AtomicUsize,
        batch_calls: AtomicUsize,
        batch_put_calls: AtomicUsize,
        batch_get_ordered_calls: AtomicUsize,
        max_batch_get_ordered_len: AtomicUsize,
    }

    impl Store for CountingStore {
        type Error = CountingStoreError;

        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
            let data = self.data.lock().unwrap();
            Ok(data.get(key).cloned())
        }

        fn put(&self, key: &[u8], value: &[u8]) -> Result<(), Self::Error> {
            let mut data = self.data.lock().unwrap();
            self.put_calls.fetch_add(1, Ordering::Relaxed);
            data.insert(key.to_vec(), value.to_vec());
            Ok(())
        }

        fn delete(&self, key: &[u8]) -> Result<(), Self::Error> {
            let mut data = self.data.lock().unwrap();
            data.remove(key);
            Ok(())
        }

        fn batch(&self, ops: &[BatchOp]) -> Result<(), Self::Error> {
            let mut data = self.data.lock().unwrap();
            self.batch_calls.fetch_add(1, Ordering::Relaxed);
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

        fn batch_put(&self, entries: &[(&[u8], &[u8])]) -> Result<(), Self::Error> {
            let mut data = self.data.lock().unwrap();
            self.batch_put_calls.fetch_add(1, Ordering::Relaxed);
            for (key, value) in entries {
                data.insert(key.to_vec(), value.to_vec());
            }
            Ok(())
        }

        fn batch_get_ordered(&self, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>, Self::Error> {
            self.batch_get_ordered_calls.fetch_add(1, Ordering::Relaxed);
            self.max_batch_get_ordered_len
                .fetch_max(keys.len(), Ordering::Relaxed);
            let data = self.data.lock().unwrap();
            Ok(keys.iter().map(|key| data.get(*key).cloned()).collect())
        }
    }

    #[test]
    fn test_prolly_new() {
        let store = MemStore::new();
        let config = Config::default();
        let _prolly = Prolly::new(store, config);
    }

    #[test]
    fn test_create_empty_tree() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        assert!(tree.is_empty());
        assert!(tree.root.is_none());
    }

    #[test]
    fn load_many_ordered_deduplicates_missing_cids() {
        let store = Arc::new(CountingStore::default());
        let prolly = Prolly::new(store.clone(), Config::default());
        let tree = prolly
            .put(&prolly.create(), b"key".to_vec(), b"value".to_vec())
            .unwrap();
        let root = tree.root.clone().unwrap();
        prolly.clear_cache();

        let nodes = prolly
            .load_many_ordered(&[root.clone(), root.clone(), root.clone()])
            .unwrap();

        assert_eq!(nodes.len(), 3);
        assert!(Arc::ptr_eq(&nodes[0], &nodes[1]));
        assert!(Arc::ptr_eq(&nodes[1], &nodes[2]));
        assert_eq!(prolly.cache_len(), 1);
        assert_eq!(store.batch_get_ordered_calls.load(Ordering::Relaxed), 1);
        assert_eq!(
            store.max_batch_get_ordered_len.load(Ordering::Relaxed),
            1,
            "duplicate CIDs should be fetched and decoded once"
        );
    }

    #[test]
    fn test_get_empty_tree() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let result = prolly.get(&tree, b"key").unwrap();
        assert_eq!(result, None);
    }

    #[test]
    fn test_put_and_get() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly
            .put(&tree, b"key1".to_vec(), b"value1".to_vec())
            .unwrap();
        let result = prolly.get(&tree, b"key1").unwrap();

        assert_eq!(result, Some(b"value1".to_vec()));
    }

    #[test]
    fn test_put_multiple_keys() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly
            .put(&tree, b"key1".to_vec(), b"value1".to_vec())
            .unwrap();
        let tree = prolly
            .put(&tree, b"key2".to_vec(), b"value2".to_vec())
            .unwrap();
        let tree = prolly
            .put(&tree, b"key3".to_vec(), b"value3".to_vec())
            .unwrap();

        assert_eq!(
            prolly.get(&tree, b"key1").unwrap(),
            Some(b"value1".to_vec())
        );
        assert_eq!(
            prolly.get(&tree, b"key2").unwrap(),
            Some(b"value2".to_vec())
        );
        assert_eq!(
            prolly.get(&tree, b"key3").unwrap(),
            Some(b"value3".to_vec())
        );
    }

    #[test]
    fn test_put_update_existing() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly
            .put(&tree, b"key".to_vec(), b"value1".to_vec())
            .unwrap();
        let tree = prolly
            .put(&tree, b"key".to_vec(), b"value2".to_vec())
            .unwrap();

        assert_eq!(prolly.get(&tree, b"key").unwrap(), Some(b"value2".to_vec()));
    }

    #[test]
    fn test_put_idempotent() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree1 = prolly
            .put(&tree, b"key".to_vec(), b"value".to_vec())
            .unwrap();
        let tree2 = prolly
            .put(&tree1, b"key".to_vec(), b"value".to_vec())
            .unwrap();

        // Same value should return same tree (same root)
        assert_eq!(tree1.root, tree2.root);
    }

    #[test]
    fn test_delete_existing_key() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly
            .put(&tree, b"key".to_vec(), b"value".to_vec())
            .unwrap();
        let tree = prolly.delete(&tree, b"key").unwrap();

        assert_eq!(prolly.get(&tree, b"key").unwrap(), None);
    }

    #[test]
    fn test_delete_nonexistent_key() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly
            .put(&tree, b"key1".to_vec(), b"value1".to_vec())
            .unwrap();
        let tree2 = prolly.delete(&tree, b"nonexistent").unwrap();

        // Should return same tree (idempotent)
        assert_eq!(tree.root, tree2.root);
    }

    #[test]
    fn test_delete_empty_tree() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree2 = prolly.delete(&tree, b"key").unwrap();

        // Should return same empty tree
        assert!(tree2.is_empty());
    }

    #[test]
    fn test_delete_last_key_makes_empty() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly
            .put(&tree, b"key".to_vec(), b"value".to_vec())
            .unwrap();
        let tree = prolly.delete(&tree, b"key").unwrap();

        assert!(tree.is_empty());
    }

    #[test]
    fn test_get_nonexistent_key() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly
            .put(&tree, b"key1".to_vec(), b"value1".to_vec())
            .unwrap();
        let result = prolly.get(&tree, b"nonexistent").unwrap();

        assert_eq!(result, None);
    }

    #[test]
    fn test_immutability() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree1 = prolly.create();

        let tree2 = prolly
            .put(&tree1, b"key".to_vec(), b"value".to_vec())
            .unwrap();

        // Original tree should still be empty
        assert!(tree1.is_empty());
        // New tree should have the key
        assert!(!tree2.is_empty());
    }

    #[test]
    fn test_keys_sorted_order() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        // Insert in reverse order
        let tree = prolly.put(&tree, b"c".to_vec(), b"3".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();

        // All keys should be retrievable
        assert_eq!(prolly.get(&tree, b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(prolly.get(&tree, b"b").unwrap(), Some(b"2".to_vec()));
        assert_eq!(prolly.get(&tree, b"c").unwrap(), Some(b"3".to_vec()));
    }

    #[test]
    fn test_put_batches_non_append_rebalance_writes() {
        let store = Arc::new(CountingStore::default());
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(4)
            .chunking_factor(1000000)
            .build();
        let prolly = Prolly::new(store.clone(), config);
        let mut tree = prolly.create();

        for i in 0..20 {
            tree = prolly
                .put(
                    &tree,
                    format!("k{i:03}").into_bytes(),
                    format!("v{i:03}").into_bytes(),
                )
                .unwrap();
        }

        let put_calls_before = store.put_calls.load(Ordering::Relaxed);
        let batch_put_calls_before = store.batch_put_calls.load(Ordering::Relaxed);

        let tree = prolly
            .put(&tree, b"k010".to_vec(), b"changed".to_vec())
            .unwrap();

        assert_eq!(
            prolly.get(&tree, b"k010").unwrap(),
            Some(b"changed".to_vec())
        );
        assert_eq!(
            store.put_calls.load(Ordering::Relaxed),
            put_calls_before,
            "non-append put should avoid per-node store.put calls"
        );
        assert_eq!(
            store.batch_put_calls.load(Ordering::Relaxed) - batch_put_calls_before,
            1,
            "non-append put should flush rewritten path in one batch"
        );
    }

    #[test]
    fn test_delete_batches_rebalance_writes() {
        let store = Arc::new(CountingStore::default());
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(4)
            .chunking_factor(1000000)
            .build();
        let prolly = Prolly::new(store.clone(), config);
        let mut tree = prolly.create();

        for i in 0..20 {
            tree = prolly
                .put(
                    &tree,
                    format!("k{i:03}").into_bytes(),
                    format!("v{i:03}").into_bytes(),
                )
                .unwrap();
        }

        let put_calls_before = store.put_calls.load(Ordering::Relaxed);
        let batch_put_calls_before = store.batch_put_calls.load(Ordering::Relaxed);

        let tree = prolly.delete(&tree, b"k010").unwrap();

        assert_eq!(prolly.get(&tree, b"k010").unwrap(), None);
        assert_eq!(
            store.put_calls.load(Ordering::Relaxed),
            put_calls_before,
            "delete should avoid per-node store.put calls"
        );
        assert_eq!(
            store.batch_put_calls.load(Ordering::Relaxed) - batch_put_calls_before,
            1,
            "delete should flush rewritten path in one batch"
        );
    }

    #[test]
    fn test_rebalance_split_on_max_chunk_size() {
        let store = MemStore::new();
        // Use a small max_chunk_size to force splits
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(4)
            .chunking_factor(1000000) // High factor to minimize hash-based splits
            .build();
        let prolly = Prolly::new(store, config);
        let tree = prolly.create();

        // Insert enough keys to trigger a split
        let mut tree = tree;
        for i in 0..10 {
            let key = format!("key{:02}", i).into_bytes();
            let val = format!("val{:02}", i).into_bytes();
            tree = prolly.put(&tree, key, val).unwrap();
        }

        // All keys should still be retrievable after splits
        for i in 0..10 {
            let key = format!("key{:02}", i).into_bytes();
            let expected = format!("val{:02}", i).into_bytes();
            assert_eq!(prolly.get(&tree, &key).unwrap(), Some(expected));
        }
    }

    #[test]
    fn test_rebalance_creates_new_root_on_split() {
        let store = MemStore::new();
        // Use a very small max_chunk_size to force root split
        let config = Config::builder()
            .min_chunk_size(1)
            .max_chunk_size(3)
            .chunking_factor(1000000) // High factor to minimize hash-based splits
            .build();
        let prolly = Prolly::new(store, config);
        let tree = prolly.create();

        // Insert enough keys to trigger root split
        let mut tree = tree;
        for i in 0..10 {
            let key = format!("k{:02}", i).into_bytes();
            let val = format!("v{:02}", i).into_bytes();
            tree = prolly.put(&tree, key, val).unwrap();
        }

        // Tree should have a root
        assert!(tree.root.is_some());

        // All keys should be retrievable
        for i in 0..10 {
            let key = format!("k{:02}", i).into_bytes();
            let expected = format!("v{:02}", i).into_bytes();
            assert_eq!(
                prolly.get(&tree, &key).unwrap(),
                Some(expected),
                "Key k{:02} not found",
                i
            );
        }
    }

    #[test]
    fn test_rebalance_propagates_changes_to_root() {
        let store = MemStore::new();
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(5)
            .chunking_factor(1000000)
            .build();
        let prolly = Prolly::new(store, config);
        let tree = prolly.create();

        // Build a tree with multiple levels
        let mut tree = tree;
        for i in 0..20 {
            let key = format!("key{:03}", i).into_bytes();
            let val = format!("val{:03}", i).into_bytes();
            tree = prolly.put(&tree, key, val).unwrap();
        }

        // Verify all keys are accessible
        for i in 0..20 {
            let key = format!("key{:03}", i).into_bytes();
            let expected = format!("val{:03}", i).into_bytes();
            assert_eq!(prolly.get(&tree, &key).unwrap(), Some(expected));
        }
    }

    #[test]
    fn test_delete_triggers_rebalance() {
        let store = MemStore::new();
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(5)
            .chunking_factor(1000000)
            .build();
        let prolly = Prolly::new(store, config);
        let tree = prolly.create();

        // Build a tree
        let mut tree = tree;
        for i in 0..10 {
            let key = format!("key{:02}", i).into_bytes();
            let val = format!("val{:02}", i).into_bytes();
            tree = prolly.put(&tree, key, val).unwrap();
        }

        // Delete some keys
        for i in 0..5 {
            let key = format!("key{:02}", i).into_bytes();
            tree = prolly.delete(&tree, &key).unwrap();
        }

        // Remaining keys should still be accessible
        for i in 5..10 {
            let key = format!("key{:02}", i).into_bytes();
            let expected = format!("val{:02}", i).into_bytes();
            assert_eq!(prolly.get(&tree, &key).unwrap(), Some(expected));
        }

        // Deleted keys should not be found
        for i in 0..5 {
            let key = format!("key{:02}", i).into_bytes();
            assert_eq!(prolly.get(&tree, &key).unwrap(), None);
        }
    }

    #[test]
    fn test_boundary_based_splitting() {
        let store = MemStore::new();
        // Use a low chunking factor to increase boundary probability
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(100)
            .chunking_factor(4) // Low factor = more boundaries
            .build();
        let prolly = Prolly::new(store, config);
        let tree = prolly.create();

        // Insert many keys
        let mut tree = tree;
        for i in 0..50 {
            let key = format!("key{:03}", i).into_bytes();
            let val = format!("val{:03}", i).into_bytes();
            tree = prolly.put(&tree, key, val).unwrap();
        }

        // All keys should be retrievable
        for i in 0..50 {
            let key = format!("key{:03}", i).into_bytes();
            let expected = format!("val{:03}", i).into_bytes();
            assert_eq!(prolly.get(&tree, &key).unwrap(), Some(expected));
        }
    }

    // ========== Range Iteration Tests ==========

    #[test]
    fn test_range_empty_tree() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let results: Vec<_> = prolly.range(&tree, &[], None).unwrap().collect();
        assert!(results.is_empty());
    }

    #[test]
    fn test_range_single_element() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly
            .put(&tree, b"key".to_vec(), b"value".to_vec())
            .unwrap();

        let results: Vec<_> = prolly
            .range(&tree, &[], None)
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0], (b"key".to_vec(), b"value".to_vec()));
    }

    #[test]
    fn test_range_all_elements() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"c".to_vec(), b"3".to_vec()).unwrap();

        let results: Vec<_> = prolly
            .range(&tree, &[], None)
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(results.len(), 3);
        assert_eq!(results[0], (b"a".to_vec(), b"1".to_vec()));
        assert_eq!(results[1], (b"b".to_vec(), b"2".to_vec()));
        assert_eq!(results[2], (b"c".to_vec(), b"3".to_vec()));
    }

    #[test]
    fn test_range_with_start_bound() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"c".to_vec(), b"3".to_vec()).unwrap();

        // Start from "b"
        let results: Vec<_> = prolly
            .range(&tree, b"b", None)
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0], (b"b".to_vec(), b"2".to_vec()));
        assert_eq!(results[1], (b"c".to_vec(), b"3".to_vec()));
    }

    #[test]
    fn test_range_with_end_bound() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"c".to_vec(), b"3".to_vec()).unwrap();

        // End at "c" (exclusive)
        let results: Vec<_> = prolly
            .range(&tree, &[], Some(b"c"))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0], (b"a".to_vec(), b"1".to_vec()));
        assert_eq!(results[1], (b"b".to_vec(), b"2".to_vec()));
    }

    #[test]
    fn test_range_with_both_bounds() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"c".to_vec(), b"3".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"d".to_vec(), b"4".to_vec()).unwrap();

        // Range [b, d)
        let results: Vec<_> = prolly
            .range(&tree, b"b", Some(b"d"))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0], (b"b".to_vec(), b"2".to_vec()));
        assert_eq!(results[1], (b"c".to_vec(), b"3".to_vec()));
    }

    #[test]
    fn test_range_start_not_found() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"c".to_vec(), b"3".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"e".to_vec(), b"5".to_vec()).unwrap();

        // Start from "b" (doesn't exist, should start at "c")
        let results: Vec<_> = prolly
            .range(&tree, b"b", None)
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(results.len(), 2);
        assert_eq!(results[0], (b"c".to_vec(), b"3".to_vec()));
        assert_eq!(results[1], (b"e".to_vec(), b"5".to_vec()));
    }

    #[test]
    fn test_range_lexicographic_order() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        // Insert in random order
        let tree = prolly.put(&tree, b"zebra".to_vec(), b"z".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"apple".to_vec(), b"a".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"mango".to_vec(), b"m".to_vec()).unwrap();
        let tree = prolly
            .put(&tree, b"banana".to_vec(), b"b".to_vec())
            .unwrap();

        let results: Vec<_> = prolly
            .range(&tree, &[], None)
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        // Should be in lexicographic order
        assert_eq!(results.len(), 4);
        assert_eq!(results[0].0, b"apple".to_vec());
        assert_eq!(results[1].0, b"banana".to_vec());
        assert_eq!(results[2].0, b"mango".to_vec());
        assert_eq!(results[3].0, b"zebra".to_vec());
    }

    #[test]
    fn test_range_across_multiple_nodes() {
        let store = MemStore::new();
        // Use small max_chunk_size to force multiple nodes
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(4)
            .chunking_factor(1000000)
            .build();
        let prolly = Prolly::new(store, config);
        let tree = prolly.create();

        // Insert enough keys to span multiple nodes
        let mut tree = tree;
        for i in 0..20 {
            let key = format!("key{:02}", i).into_bytes();
            let val = format!("val{:02}", i).into_bytes();
            tree = prolly.put(&tree, key, val).unwrap();
        }

        // Iterate over all and verify order
        let results: Vec<_> = prolly
            .range(&tree, &[], None)
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert_eq!(results.len(), 20);

        // Verify keys are in order
        for (i, item) in results.iter().enumerate().take(20) {
            let expected_key = format!("key{:02}", i).into_bytes();
            let expected_val = format!("val{:02}", i).into_bytes();
            assert_eq!(item, &(expected_key, expected_val));
        }
    }

    #[test]
    fn test_range_empty_result() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();

        // Range that doesn't match anything
        let results: Vec<_> = prolly
            .range(&tree, b"c", Some(b"d"))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert!(results.is_empty());
    }

    #[test]
    fn test_range_start_past_all_keys() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();

        // Start past all keys
        let results: Vec<_> = prolly
            .range(&tree, b"z", None)
            .unwrap()
            .map(|r| r.unwrap())
            .collect();

        assert!(results.is_empty());
    }

    // ========== Diff Tests ==========

    #[test]
    fn test_diff_same_tree() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();

        // Diff of same tree should be empty
        let diffs = prolly.diff(&tree, &tree).unwrap();
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_diff_empty_trees() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();
        let other = prolly.create();

        // Diff of two empty trees should be empty
        let diffs = prolly.diff(&base, &other).unwrap();
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_diff_added_entries() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let other = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();

        let diffs = prolly.diff(&base, &other).unwrap();

        // Should have one Added entry
        assert_eq!(diffs.len(), 1);
        assert!(matches!(
            &diffs[0],
            Diff::Added { key, val } if key == b"b" && val == b"2"
        ));
    }

    #[test]
    fn test_diff_removed_entries() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let base = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
        let other = prolly.delete(&base, b"b").unwrap();

        let diffs = prolly.diff(&base, &other).unwrap();

        // Should have one Removed entry
        assert_eq!(diffs.len(), 1);
        assert!(matches!(
            &diffs[0],
            Diff::Removed { key, val } if key == b"b" && val == b"2"
        ));
    }

    #[test]
    fn test_diff_changed_entries() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let other = prolly.put(&base, b"a".to_vec(), b"2".to_vec()).unwrap();

        let diffs = prolly.diff(&base, &other).unwrap();

        // Should have one Changed entry
        assert_eq!(diffs.len(), 1);
        assert!(matches!(
            &diffs[0],
            Diff::Changed { key, old, new } if key == b"a" && old == b"1" && new == b"2"
        ));
    }

    #[test]
    fn test_diff_mixed_changes() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        // Base tree: a=1, b=2, c=3
        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let base = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
        let base = prolly.put(&base, b"c".to_vec(), b"3".to_vec()).unwrap();

        // Other tree: a=1 (unchanged), b=X (changed), d=4 (added), c removed
        let other = prolly.put(&base, b"b".to_vec(), b"X".to_vec()).unwrap();
        let other = prolly.put(&other, b"d".to_vec(), b"4".to_vec()).unwrap();
        let other = prolly.delete(&other, b"c").unwrap();

        let diffs = prolly.diff(&base, &other).unwrap();

        // Should have 3 diffs: Changed(b), Added(d), Removed(c)
        assert_eq!(diffs.len(), 3);

        // Check for Changed entry
        assert!(diffs.iter().any(|d| matches!(
            d,
            Diff::Changed { key, old, new } if key == b"b" && old == b"2" && new == b"X"
        )));

        // Check for Added entry
        assert!(diffs.iter().any(|d| matches!(
            d,
            Diff::Added { key, val } if key == b"d" && val == b"4"
        )));

        // Check for Removed entry
        assert!(diffs.iter().any(|d| matches!(
            d,
            Diff::Removed { key, val } if key == b"c" && val == b"3"
        )));
    }

    #[test]
    fn test_diff_from_empty_base() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let other = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let other = prolly.put(&other, b"b".to_vec(), b"2".to_vec()).unwrap();

        let diffs = prolly.diff(&base, &other).unwrap();

        // All entries in other should be Added
        assert_eq!(diffs.len(), 2);
        assert!(diffs.iter().all(|d| matches!(d, Diff::Added { .. })));
    }

    #[test]
    fn test_diff_to_empty_other() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let base = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();

        let other = prolly.create();

        let diffs = prolly.diff(&base, &other).unwrap();

        // All entries in base should be Removed
        assert_eq!(diffs.len(), 2);
        assert!(diffs.iter().all(|d| matches!(d, Diff::Removed { .. })));
    }

    #[test]
    fn test_diff_no_changes() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();

        // Create other by putting same key-value (should have same root due to idempotence)
        let other = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();

        let diffs = prolly.diff(&base, &other).unwrap();

        // Should be empty since trees are identical
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_diff_across_multiple_nodes() {
        let store = MemStore::new();
        // Use default config to avoid triggering rebalancing edge cases
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        // Build base tree
        let mut base = base;
        for i in 0..10 {
            let key = format!("key{:02}", i).into_bytes();
            let val = format!("val{:02}", i).into_bytes();
            base = prolly.put(&base, key, val).unwrap();
        }

        // Create other with some changes starting from base
        // Change key05
        let other = prolly
            .put(&base, b"key05".to_vec(), b"changed".to_vec())
            .unwrap();
        // Add key10
        let other = prolly
            .put(&other, b"key10".to_vec(), b"val10".to_vec())
            .unwrap();

        let diffs = prolly.diff(&base, &other).unwrap();

        // Should have 2 diffs: Changed(key05), Added(key10)
        assert_eq!(diffs.len(), 2);

        // Check for Changed entry
        assert!(diffs.iter().any(|d| matches!(
            d,
            Diff::Changed { key, old, new } if key == b"key05" && old == b"val05" && new == b"changed"
        )), "Expected Changed entry for key05");

        // Check for Added entry
        assert!(
            diffs.iter().any(|d| matches!(
                d,
                Diff::Added { key, val } if key == b"key10" && val == b"val10"
            )),
            "Expected Added entry for key10"
        );
    }

    // ========== Merge Tests ==========

    #[test]
    fn test_merge_no_changes() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();

        // Both branches are identical to base
        let merged = prolly.merge(&base, &base, &base, None).unwrap();

        // Merged should be same as base
        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"1".to_vec()));
    }

    #[test]
    fn test_merge_only_left_changes() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let left = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();

        // Right is same as base
        let merged = prolly.merge(&base, &left, &base, None).unwrap();

        // Merged should have both keys
        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(prolly.get(&merged, b"b").unwrap(), Some(b"2".to_vec()));
    }

    #[test]
    fn test_merge_only_right_changes() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let right = prolly.put(&base, b"c".to_vec(), b"3".to_vec()).unwrap();

        // Left is same as base
        let merged = prolly.merge(&base, &base, &right, None).unwrap();

        // Merged should have both keys
        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(prolly.get(&merged, b"c").unwrap(), Some(b"3".to_vec()));
    }

    #[test]
    fn test_merge_both_add_different_keys() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let left = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
        let right = prolly.put(&base, b"c".to_vec(), b"3".to_vec()).unwrap();

        // No conflict - different keys
        let merged = prolly.merge(&base, &left, &right, None).unwrap();

        // Merged should have all keys
        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(prolly.get(&merged, b"b").unwrap(), Some(b"2".to_vec()));
        assert_eq!(prolly.get(&merged, b"c").unwrap(), Some(b"3".to_vec()));
    }

    #[test]
    fn test_merge_both_add_same_key_same_value() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let left = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
        let right = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();

        // No conflict - same key with same value
        let merged = prolly.merge(&base, &left, &right, None).unwrap();

        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(prolly.get(&merged, b"b").unwrap(), Some(b"2".to_vec()));
    }

    #[test]
    fn test_merge_conflict_without_resolver() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let left = prolly.put(&base, b"a".to_vec(), b"left".to_vec()).unwrap();
        let right = prolly.put(&base, b"a".to_vec(), b"right".to_vec()).unwrap();

        // Conflict - same key with different values, no resolver
        let result = prolly.merge(&base, &left, &right, None);

        assert!(matches!(result, Err(Error::Conflict(_))));
        if let Err(Error::Conflict(conflict)) = result {
            assert_eq!(conflict.key, b"a".to_vec());
            assert_eq!(conflict.base, Some(b"1".to_vec()));
            assert_eq!(conflict.left, b"left".to_vec());
            assert_eq!(conflict.right, b"right".to_vec());
        }
    }

    #[test]
    fn test_merge_conflict_with_resolver_prefer_left() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let left = prolly.put(&base, b"a".to_vec(), b"left".to_vec()).unwrap();
        let right = prolly.put(&base, b"a".to_vec(), b"right".to_vec()).unwrap();

        // Resolver that prefers left
        let resolver: error::Resolver = Box::new(|c| Some(c.left.clone()));
        let merged = prolly.merge(&base, &left, &right, Some(resolver)).unwrap();

        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"left".to_vec()));
    }

    #[test]
    fn test_merge_conflict_with_resolver_prefer_right() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let left = prolly.put(&base, b"a".to_vec(), b"left".to_vec()).unwrap();
        let right = prolly.put(&base, b"a".to_vec(), b"right".to_vec()).unwrap();

        // Resolver that prefers right
        let resolver: error::Resolver = Box::new(|c| Some(c.right.clone()));
        let merged = prolly.merge(&base, &left, &right, Some(resolver)).unwrap();

        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"right".to_vec()));
    }

    #[test]
    fn test_merge_conflict_with_resolver_returns_none() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let left = prolly.put(&base, b"a".to_vec(), b"left".to_vec()).unwrap();
        let right = prolly.put(&base, b"a".to_vec(), b"right".to_vec()).unwrap();

        // Resolver that returns None
        let resolver: error::Resolver = Box::new(|_| None);
        let result = prolly.merge(&base, &left, &right, Some(resolver));

        assert!(matches!(result, Err(Error::Conflict(_))));
    }

    #[test]
    fn test_merge_left_deletes_right_modifies() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let left = prolly.delete(&base, b"a").unwrap();
        let right = prolly
            .put(&base, b"a".to_vec(), b"modified".to_vec())
            .unwrap();

        // Conflict - left deletes, right modifies
        let result = prolly.merge(&base, &left, &right, None);

        assert!(matches!(result, Err(Error::Conflict(_))));
    }

    #[test]
    fn test_merge_right_deletes_only() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let base = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
        let right = prolly.delete(&base, b"b").unwrap();

        // Left is same as base, right deletes b
        let merged = prolly.merge(&base, &base, &right, None).unwrap();

        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(prolly.get(&merged, b"b").unwrap(), None);
    }

    #[test]
    fn test_merge_both_delete_same_key() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let base = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
        let left = prolly.delete(&base, b"b").unwrap();
        let right = prolly.delete(&base, b"b").unwrap();

        // Both delete same key - no conflict
        let merged = prolly.merge(&base, &left, &right, None).unwrap();

        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(prolly.get(&merged, b"b").unwrap(), None);
    }

    #[test]
    fn test_merge_empty_base() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let left = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let right = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();

        let merged = prolly.merge(&base, &left, &right, None).unwrap();

        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(prolly.get(&merged, b"b").unwrap(), Some(b"2".to_vec()));
    }

    #[test]
    fn test_merge_complex_scenario() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        // Base: a=1, b=2, c=3, d=4
        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let base = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
        let base = prolly.put(&base, b"c".to_vec(), b"3".to_vec()).unwrap();
        let base = prolly.put(&base, b"d".to_vec(), b"4".to_vec()).unwrap();

        // Left: a=1 (unchanged), b=left (modified), c deleted, e=5 (added)
        let left = prolly.put(&base, b"b".to_vec(), b"left".to_vec()).unwrap();
        let left = prolly.delete(&left, b"c").unwrap();
        let left = prolly.put(&left, b"e".to_vec(), b"5".to_vec()).unwrap();

        // Right: a=1 (unchanged), d=right (modified), f=6 (added)
        let right = prolly.put(&base, b"d".to_vec(), b"right".to_vec()).unwrap();
        let right = prolly.put(&right, b"f".to_vec(), b"6".to_vec()).unwrap();

        let merged = prolly.merge(&base, &left, &right, None).unwrap();

        // Check merged state
        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"1".to_vec())); // unchanged
        assert_eq!(prolly.get(&merged, b"b").unwrap(), Some(b"left".to_vec())); // left modified
        assert_eq!(prolly.get(&merged, b"c").unwrap(), None); // left deleted
        assert_eq!(prolly.get(&merged, b"d").unwrap(), Some(b"right".to_vec())); // right modified
        assert_eq!(prolly.get(&merged, b"e").unwrap(), Some(b"5".to_vec())); // left added
        assert_eq!(prolly.get(&merged, b"f").unwrap(), Some(b"6".to_vec())); // right added
    }
}
