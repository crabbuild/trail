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

use rayon::prelude::*;
use std::collections::{hash_map::Entry, HashMap};
use std::ops::Range;
use std::sync::{Arc, RwLock};

const PARALLEL_NODE_DECODE_THRESHOLD: usize = 16;
const GET_MANY_PREFETCH_PARALLELISM: usize = 16;
const GET_MANY_BOUNDARY_ROUTE_MIN_POSITIONS: usize = 32;
const STATS_FRONTIER_PREFETCH_PARALLELISM: usize = 16;

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

struct KeyLookupFrame {
    cid: Cid,
    positions: InlinePositions,
}

#[derive(Default)]
struct MissingNodeBatch {
    indexes: HashMap<Cid, usize>,
    cids: Vec<Cid>,
    positions: Vec<InlinePositions>,
}

struct InlinePositions {
    first: usize,
    rest: Vec<usize>,
}

impl InlinePositions {
    fn new(first: usize) -> Self {
        Self {
            first,
            rest: Vec::new(),
        }
    }

    fn with_rest_capacity(first: usize, rest_capacity: usize) -> Self {
        Self {
            first,
            rest: Vec::with_capacity(rest_capacity),
        }
    }

    fn from_vec(positions: Vec<usize>) -> Option<Self> {
        let mut iter = positions.into_iter();
        let first = iter.next()?;
        Some(Self {
            first,
            rest: iter.collect(),
        })
    }

    fn push(&mut self, position: usize) {
        self.rest.push(position);
    }

    fn len(&self) -> usize {
        1 + self.rest.len()
    }

    fn at(&self, offset: usize) -> usize {
        if offset == 0 {
            self.first
        } else {
            self.rest[offset - 1]
        }
    }
}

impl IntoIterator for InlinePositions {
    type Item = usize;
    type IntoIter = std::iter::Chain<std::iter::Once<usize>, std::vec::IntoIter<usize>>;

    fn into_iter(self) -> Self::IntoIter {
        std::iter::once(self.first).chain(self.rest)
    }
}

impl MissingNodeBatch {
    fn with_capacity(capacity: usize) -> Self {
        Self {
            indexes: HashMap::with_capacity(capacity),
            cids: Vec::with_capacity(capacity),
            positions: Vec::with_capacity(capacity),
        }
    }

    fn record(&mut self, cid: &Cid, position: usize) {
        match self.indexes.entry(cid.clone()) {
            Entry::Occupied(entry) => {
                self.positions[*entry.get()].push(position);
            }
            Entry::Vacant(entry) => {
                let missing_idx = self.cids.len();
                self.cids.push(cid.clone());
                self.positions.push(InlinePositions::new(position));
                entry.insert(missing_idx);
            }
        }
    }
}

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
pub use batch::BatchApplyResult;
pub use batch::BatchApplyStats;
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
                    return Ok(Some(leaf_value_at(&node, idx)?));
                }
                return Ok(None);
            }

            // Descend to child
            cid = child_cid_at(&node, idx)?;
        }
    }

    /// Get multiple values from the tree while preserving caller order.
    ///
    /// This descends the tree level-by-level and batches node loads for the
    /// current lookup frontier. It is useful for random read-after-write
    /// verification and merge conflict checks because shared ancestors and
    /// leaves are loaded once instead of once per key.
    ///
    /// Duplicate keys are allowed; each output slot corresponds to the input
    /// key at the same index.
    ///
    /// # Arguments
    /// * `tree` - The tree to search
    /// * `keys` - Keys to look up
    ///
    /// # Returns
    /// A vector of values in the same order as `keys`.
    ///
    /// # Example
    /// ```rust
    /// use prolly::{Config, MemStore, Prolly};
    ///
    /// let prolly = Prolly::new(MemStore::new(), Config::default());
    /// let tree = prolly.create();
    /// let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
    /// let tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();
    ///
    /// let values = prolly.get_many(&tree, &[b"b".to_vec(), b"missing".to_vec(), b"a".to_vec()]).unwrap();
    /// assert_eq!(values, vec![Some(b"2".to_vec()), None, Some(b"1".to_vec())]);
    /// ```
    pub fn get_many<K: AsRef<[u8]>>(
        &self,
        tree: &Tree,
        keys: &[K],
    ) -> Result<Vec<Option<Vec<u8>>>, Error> {
        let mut values = vec![None; keys.len()];
        let Some(root_cid) = &tree.root else {
            return Ok(values);
        };

        if keys.is_empty() {
            return Ok(values);
        }

        let positions = InlinePositions::from_vec(sorted_key_positions(keys))
            .expect("keys is non-empty after early return");

        let mut frames = vec![KeyLookupFrame {
            cid: root_cid.clone(),
            positions,
        }];

        while !frames.is_empty() {
            let cids = frames
                .iter()
                .map(|frame| frame.cid.clone())
                .collect::<Vec<_>>();
            let nodes = if self.store.prefers_batch_reads() {
                self.load_many_ordered_with_parallelism(&cids, GET_MANY_PREFETCH_PARALLELISM)?
            } else {
                self.load_many_ordered(&cids)?
            };
            let mut next_frames = Vec::new();

            for (frame, node) in frames.into_iter().zip(nodes) {
                if node.leaf {
                    fill_leaf_lookup_values(&node, frame.positions, keys, &mut values)?;
                    continue;
                }

                next_frames.extend(route_key_positions_to_children(
                    &node,
                    frame.positions,
                    keys,
                )?);
            }

            frames = next_frames;
        }

        Ok(values)
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

        self.collect_stats_from_frontier(root_cid, &mut stats)?;

        // Finalize statistics
        stats.finalize();

        // Return result
        Ok(stats)
    }

    /// Frontier helper for collecting statistics
    ///
    /// Traverses the tree level-by-level, accumulating statistics at each node
    /// and batching each frontier's child loads when the store supports it.
    ///
    /// # Arguments
    /// * `root_cid` - The CID of the root node
    /// * `stats` - Mutable reference to the statistics accumulator
    ///
    /// # Returns
    /// * `Ok(())` - Statistics accumulated successfully
    /// * `Err(Error)` - On storage or deserialization errors
    fn collect_stats_from_frontier(
        &self,
        root_cid: &Cid,
        stats: &mut TreeStats,
    ) -> Result<(), Error> {
        let parallelism = if self.store.prefers_batch_reads() {
            STATS_FRONTIER_PREFETCH_PARALLELISM
        } else {
            1
        };
        let mut frontier = vec![root_cid.clone()];

        while !frontier.is_empty() {
            let nodes = self.load_many_ordered_with_parallelism(&frontier, parallelism)?;
            let mut next_frontier = Vec::new();

            for node in nodes {
                if node.keys.len() != node.vals.len() {
                    return Err(Error::InvalidNode);
                }
                stats.accumulate(&node);

                if !node.leaf {
                    next_frontier.reserve(node.vals.len());
                    for idx in 0..node.len() {
                        next_frontier.push(child_cid_at(&node, idx)?);
                    }
                }
            }

            frontier = next_frontier;
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
        if end.is_some_and(|end| end <= start) {
            return Ok(cursor::CursorIterator::with_bounds(
                cursor::Cursor::invalid(),
                &self.store,
                Some(start.to_vec()),
                end.map(|e| e.to_vec()),
            ));
        }

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
        self.load_many_ordered_with_parallelism(cids, 1)
    }

    /// Load nodes by CID in input order, splitting cache misses across up to
    /// `parallelism` concurrent ordered batch reads.
    pub(crate) fn load_many_ordered_with_parallelism(
        &self,
        cids: &[Cid],
        parallelism: usize,
    ) -> Result<Vec<Arc<Node>>, Error> {
        if cids.is_empty() {
            return Ok(Vec::new());
        }

        let mut nodes: Vec<Option<Arc<Node>>>;
        let mut missing: Option<MissingNodeBatch>;
        if let Ok(cache) = self.node_cache.read() {
            let mut cached_nodes = Vec::with_capacity(cids.len());
            let mut first_miss = None;
            for (idx, cid) in cids.iter().enumerate() {
                if let Some(node) = cache.get(cid) {
                    cached_nodes.push(node.clone());
                } else {
                    first_miss = Some(idx);
                    break;
                }
            }

            let Some(first_miss) = first_miss else {
                return Ok(cached_nodes);
            };

            nodes = Vec::with_capacity(cids.len());
            nodes.extend(cached_nodes.into_iter().map(Some));
            nodes.resize_with(cids.len(), || None);
            missing = Some(MissingNodeBatch::with_capacity(cids.len() - first_miss));
            if let Some(missing_batch) = missing.as_mut() {
                missing_batch.record(&cids[first_miss], first_miss);
                for (idx, cid) in cids.iter().enumerate().skip(first_miss + 1) {
                    if let Some(node) = cache.get(cid) {
                        nodes[idx] = Some(node.clone());
                    } else {
                        missing_batch.record(cid, idx);
                    }
                }
            }
        } else {
            nodes = vec![None; cids.len()];
            let mut missing_batch = MissingNodeBatch::with_capacity(cids.len());
            for (idx, cid) in cids.iter().enumerate() {
                missing_batch.record(cid, idx);
            }
            missing = Some(missing_batch);
        }

        if let Some(MissingNodeBatch {
            cids: missing_cids,
            positions: missing_positions,
            ..
        }) = missing
        {
            if missing_cids.len() == 1 && !self.store.prefers_batch_reads() {
                let node = self.load_arc(&missing_cids[0])?;
                let positions = missing_positions
                    .into_iter()
                    .next()
                    .ok_or(Error::InvalidNode)?;
                for idx in positions {
                    nodes[idx] = Some(node.clone());
                }

                return nodes
                    .into_iter()
                    .collect::<Option<Vec<_>>>()
                    .ok_or(Error::InvalidNode);
            }

            let loaded = if parallelism <= 1 || missing_cids.len() <= parallelism {
                let keys = missing_cids
                    .iter()
                    .map(|cid| cid.as_bytes())
                    .collect::<Vec<_>>();
                let loaded = self
                    .store
                    .batch_get_ordered_unique(&keys)
                    .map_err(|e| Error::Store(Box::new(e)))?;
                if loaded.len() != missing_cids.len() {
                    return Err(Error::InvalidNode);
                }
                loaded
            } else {
                let chunk_size = missing_cids.len().div_ceil(parallelism);
                missing_cids
                    .par_chunks(chunk_size)
                    .map(|chunk| {
                        let keys = chunk.iter().map(|cid| cid.as_bytes()).collect::<Vec<_>>();
                        let loaded = self
                            .store
                            .batch_get_ordered_unique(&keys)
                            .map_err(|e| Error::Store(Box::new(e)))?;
                        if loaded.len() != chunk.len() {
                            return Err(Error::InvalidNode);
                        }
                        Ok(loaded)
                    })
                    .collect::<Result<Vec<_>, Error>>()?
                    .into_iter()
                    .flatten()
                    .collect::<Vec<_>>()
            };

            let decoded = if loaded.len() >= PARALLEL_NODE_DECODE_THRESHOLD {
                missing_cids
                    .into_par_iter()
                    .zip(loaded.into_par_iter())
                    .map(|(cid, bytes)| {
                        let bytes = bytes.ok_or_else(|| Error::NotFound(cid.clone()))?;
                        let node = Arc::new(Node::from_bytes(&bytes)?);
                        Ok((cid, node))
                    })
                    .collect::<Result<Vec<_>, Error>>()?
            } else {
                missing_cids
                    .into_iter()
                    .zip(loaded)
                    .map(|(cid, bytes)| {
                        let bytes = bytes.ok_or_else(|| Error::NotFound(cid.clone()))?;
                        let node = Arc::new(Node::from_bytes(&bytes)?);
                        Ok((cid, node))
                    })
                    .collect::<Result<Vec<_>, Error>>()?
            };

            let mut cache = self.node_cache.write().ok();
            for ((cid, node), positions) in decoded.into_iter().zip(missing_positions) {
                let node = if let Some(cache) = cache.as_mut() {
                    cache
                        .entry(cid.clone())
                        .or_insert_with(|| node.clone())
                        .clone()
                } else {
                    node
                };
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

    pub(crate) fn cached_node_arc(&self, cid: &Cid) -> Option<Arc<Node>> {
        self.node_cache
            .read()
            .ok()
            .and_then(|cache| cache.get(cid).cloned())
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

            cid = child_cid_at(&node, idx)?;
        }

        Ok(path)
    }

    /// Find the path from root to the key using shared cached nodes.
    pub(crate) fn find_path_arcs(
        &self,
        tree: &Tree,
        key: &[u8],
    ) -> Result<Vec<(Arc<Node>, usize)>, Error> {
        let mut path = Vec::new();

        let Some(root_cid) = &tree.root else {
            return Ok(path);
        };

        let mut cid = root_cid.clone();
        loop {
            let node = self.load_arc(&cid)?;
            let idx = match node.search(key) {
                Ok(i) => i,
                Err(i) => i.saturating_sub(1),
            };

            path.push((node.clone(), idx));

            if node.leaf {
                break;
            }

            cid = child_cid_at(&node, idx)?;
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

fn fill_leaf_lookup_values<K: AsRef<[u8]>>(
    node: &Node,
    positions: InlinePositions,
    keys: &[K],
    values: &mut [Option<Vec<u8>>],
) -> Result<(), Error> {
    if node.keys.len() != node.vals.len() {
        return Err(Error::InvalidNode);
    }

    let mut leaf_idx = 0usize;
    let mut positions = positions.into_iter().peekable();
    while let Some(position) = positions.next() {
        let key = keys[position].as_ref();

        while leaf_idx < node.keys.len() && node.keys[leaf_idx].as_slice() < key {
            leaf_idx += 1;
        }

        let found_value = if leaf_idx < node.keys.len() && node.keys[leaf_idx].as_slice() == key {
            Some(&node.vals[leaf_idx])
        } else {
            None
        };

        if let Some(value) = found_value {
            values[position] = Some(value.clone());
        }

        while let Some(next_position) =
            positions.next_if(|next_position| keys[*next_position].as_ref() == key)
        {
            if let Some(value) = found_value {
                values[next_position] = Some(value.clone());
            }
        }
    }

    Ok(())
}

fn sorted_key_positions<K: AsRef<[u8]>>(keys: &[K]) -> Vec<usize> {
    let mut positions = (0..keys.len()).collect::<Vec<_>>();
    if keys_are_sorted(keys) {
        return positions;
    }

    positions.sort_by(|left, right| {
        keys[*left]
            .as_ref()
            .cmp(keys[*right].as_ref())
            .then_with(|| left.cmp(right))
    });
    positions
}

fn keys_are_sorted<K: AsRef<[u8]>>(keys: &[K]) -> bool {
    keys.windows(2)
        .all(|pair| pair[0].as_ref() <= pair[1].as_ref())
}

fn route_key_positions_to_children<K: AsRef<[u8]>>(
    node: &Node,
    positions: InlinePositions,
    keys: &[K],
) -> Result<Vec<KeyLookupFrame>, Error> {
    if node.is_empty() {
        return Err(Error::InvalidNode);
    }

    if positions.len() >= GET_MANY_BOUNDARY_ROUTE_MIN_POSITIONS && node.len() > 1 {
        return route_key_positions_to_children_by_boundary(node, positions, keys);
    }

    let mut frames: Vec<KeyLookupFrame> = Vec::with_capacity(node.len().min(positions.len()));
    let mut child_index = child_index_for_lookup_key(node, keys[positions.first].as_ref());
    let mut last_child_index = None;

    for position in positions {
        let key = keys[position].as_ref();
        while child_index + 1 < node.len() && key >= node.keys[child_index + 1].as_slice() {
            child_index += 1;
        }

        if last_child_index == Some(child_index) {
            let frame = frames.last_mut().ok_or(Error::InvalidNode)?;
            frame.positions.push(position);
        } else {
            frames.push(KeyLookupFrame {
                cid: child_cid_at(node, child_index)?,
                positions: InlinePositions::new(position),
            });
            last_child_index = Some(child_index);
        }
    }

    Ok(frames)
}

fn route_key_positions_to_children_by_boundary<K: AsRef<[u8]>>(
    node: &Node,
    positions: InlinePositions,
    keys: &[K],
) -> Result<Vec<KeyLookupFrame>, Error> {
    let position_count = positions.len();
    let mut frames = Vec::with_capacity(node.len().min(position_count));
    let mut child_index = child_index_for_lookup_key(node, keys[positions.at(0)].as_ref());
    let last_child_index =
        child_index_for_lookup_key(node, keys[positions.at(position_count - 1)].as_ref());
    let mut bucket_start = 0usize;

    while child_index < last_child_index {
        let boundary = node.keys.get(child_index + 1).ok_or(Error::InvalidNode)?;
        let bucket_end = lower_bound_position_key(
            &positions,
            keys,
            bucket_start..position_count,
            boundary.as_slice(),
        );

        if bucket_start < bucket_end {
            frames.push(KeyLookupFrame {
                cid: child_cid_at(node, child_index)?,
                positions: inline_positions_from_range(&positions, bucket_start..bucket_end),
            });
        }

        bucket_start = bucket_end;
        child_index += 1;
    }

    if bucket_start < position_count {
        frames.push(KeyLookupFrame {
            cid: child_cid_at(node, last_child_index)?,
            positions: inline_positions_from_range(&positions, bucket_start..position_count),
        });
    }

    Ok(frames)
}

fn lower_bound_position_key<K: AsRef<[u8]>>(
    positions: &InlinePositions,
    keys: &[K],
    range: Range<usize>,
    key: &[u8],
) -> usize {
    let mut left = range.start;
    let mut right = range.end;

    while left < right {
        let mid = left + (right - left) / 2;
        if keys[positions.at(mid)].as_ref() < key {
            left = mid + 1;
        } else {
            right = mid;
        }
    }

    left
}

fn inline_positions_from_range(
    positions: &InlinePositions,
    range: Range<usize>,
) -> InlinePositions {
    debug_assert!(range.start < range.end);
    let first = positions.at(range.start);
    let mut bucket = InlinePositions::with_rest_capacity(first, range.end - range.start - 1);

    for offset in range.start + 1..range.end {
        bucket.push(positions.at(offset));
    }

    bucket
}

fn child_index_for_lookup_key(node: &Node, key: &[u8]) -> usize {
    node.keys
        .partition_point(|candidate| candidate.as_slice() <= key)
        .saturating_sub(1)
}

fn leaf_value_at(node: &Node, idx: usize) -> Result<Vec<u8>, Error> {
    node.vals.get(idx).cloned().ok_or(Error::InvalidNode)
}

fn child_cid_at(node: &Node, idx: usize) -> Result<Cid, Error> {
    let child = node.vals.get(idx).ok_or(Error::InvalidNode)?;
    Ok(Cid(child
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidNode)?))
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
        prefer_batch_reads: bool,
        get_calls: AtomicUsize,
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
            self.get_calls.fetch_add(1, Ordering::Relaxed);
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

        fn prefers_batch_reads(&self) -> bool {
            self.prefer_batch_reads
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
    fn missing_node_batch_keeps_unique_positions_inline() {
        let cid_a = Cid::from_bytes(b"a");
        let cid_b = Cid::from_bytes(b"b");
        let mut batch = MissingNodeBatch::with_capacity(2);

        batch.record(&cid_a, 3);
        batch.record(&cid_b, 9);

        assert_eq!(batch.cids, vec![cid_a.clone(), cid_b]);
        assert_eq!(batch.positions[0].first, 3);
        assert_eq!(batch.positions[0].rest.capacity(), 0);
        assert_eq!(batch.positions[1].first, 9);
        assert_eq!(batch.positions[1].rest.capacity(), 0);

        batch.record(&cid_a, 11);
        assert_eq!(
            batch.positions.remove(0).into_iter().collect::<Vec<_>>(),
            vec![3, 11]
        );
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
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            1,
            "duplicate CIDs should collapse to one point read"
        );
        assert_eq!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed),
            0,
            "single-CID miss batches should not pay ordered batch overhead"
        );
    }

    #[test]
    fn load_many_ordered_serves_cache_hits_without_store_reads() {
        let store = Arc::new(CountingStore::default());
        let prolly = Prolly::new(store.clone(), Config::default());
        let mut cids = Vec::new();

        for idx in 0..3 {
            let mut node = Node::new_leaf();
            node.keys.push(format!("k{idx:02}").into_bytes());
            node.vals.push(format!("v{idx:02}").into_bytes());
            cids.push(prolly.save(&node).unwrap());
        }

        let calls_before = store.batch_get_ordered_calls.load(Ordering::Relaxed);
        let nodes = prolly
            .load_many_ordered_with_parallelism(
                &[cids[2].clone(), cids[0].clone(), cids[2].clone()],
                4,
            )
            .unwrap();

        assert_eq!(nodes.len(), 3);
        assert_eq!(nodes[0].keys[0], b"k02".to_vec());
        assert_eq!(nodes[1].keys[0], b"k00".to_vec());
        assert!(Arc::ptr_eq(&nodes[0], &nodes[2]));
        assert_eq!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed),
            calls_before,
            "all-cache-hit frontiers should not allocate miss work or call the store"
        );
    }

    #[test]
    fn load_many_ordered_reuses_cached_prefix_and_deduplicates_later_misses() {
        let store = Arc::new(CountingStore::default());
        let prolly = Prolly::new(store.clone(), Config::default());
        let mut nodes_to_cache = Vec::new();
        let mut cids = Vec::new();

        for idx in 0..3 {
            let mut node = Node::new_leaf();
            node.keys.push(format!("k{idx:02}").into_bytes());
            node.vals.push(format!("v{idx:02}").into_bytes());
            cids.push(prolly.save(&node).unwrap());
            nodes_to_cache.push(node);
        }

        prolly.clear_cache();
        prolly.cache_node(cids[0].clone(), nodes_to_cache[0].clone());
        prolly.cache_node(cids[2].clone(), nodes_to_cache[2].clone());

        let loaded = prolly
            .load_many_ordered(&[
                cids[0].clone(),
                cids[1].clone(),
                cids[0].clone(),
                cids[2].clone(),
                cids[1].clone(),
            ])
            .unwrap();

        assert_eq!(loaded.len(), 5);
        assert_eq!(loaded[0].keys[0], b"k00".to_vec());
        assert_eq!(loaded[1].keys[0], b"k01".to_vec());
        assert_eq!(loaded[3].keys[0], b"k02".to_vec());
        assert!(Arc::ptr_eq(&loaded[0], &loaded[2]));
        assert!(Arc::ptr_eq(&loaded[1], &loaded[4]));
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            1,
            "only the single cold CID should be point-read, even when it appears twice"
        );
        assert_eq!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed),
            0,
            "default stores should avoid ordered batch overhead for one cold CID"
        );
    }

    #[test]
    fn load_many_ordered_unique_misses_use_point_reads_for_non_batched_stores() {
        let store = Arc::new(CountingStore::default());
        let prolly = Prolly::new(store.clone(), Config::default());
        let mut cids = Vec::new();

        for idx in 0..3 {
            let mut node = Node::new_leaf();
            node.keys.push(format!("k{idx:02}").into_bytes());
            node.vals.push(format!("v{idx:02}").into_bytes());
            cids.push(prolly.save(&node).unwrap());
        }
        prolly.clear_cache();

        let loaded = prolly.load_many_ordered(&cids).unwrap();

        assert_eq!(loaded.len(), cids.len());
        for (idx, node) in loaded.iter().enumerate() {
            assert_eq!(node.keys[0], format!("k{idx:02}").into_bytes());
        }
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            cids.len(),
            "point-read stores should avoid duplicate ordered-batch planning for unique misses"
        );
        assert_eq!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed),
            0,
            "point-read stores should not route already-unique misses through ordered batch reads"
        );
    }

    #[test]
    fn load_many_ordered_with_parallelism_splits_wide_misses() {
        let store = Arc::new(CountingStore {
            prefer_batch_reads: true,
            ..CountingStore::default()
        });
        let prolly = Prolly::new(store.clone(), Config::default());
        let mut cids = Vec::new();

        for idx in 0..12 {
            let mut node = Node::new_leaf();
            node.keys.push(format!("k{idx:02}").into_bytes());
            node.vals.push(format!("v{idx:02}").into_bytes());
            cids.push(prolly.save(&node).unwrap());
        }
        prolly.clear_cache();

        let nodes = prolly.load_many_ordered_with_parallelism(&cids, 3).unwrap();

        assert_eq!(nodes.len(), cids.len());
        for (idx, node) in nodes.iter().enumerate() {
            assert_eq!(node.keys[0], format!("k{idx:02}").into_bytes());
        }
        assert_eq!(prolly.cache_len(), cids.len());
        assert_eq!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed),
            3,
            "12 misses with parallelism 3 should split into 3 ordered batch reads"
        );
        assert_eq!(
            store.max_batch_get_ordered_len.load(Ordering::Relaxed),
            4,
            "wide miss sets should be split into roughly even ordered batches"
        );
    }

    #[test]
    fn load_many_ordered_parallel_decode_preserves_order_cache_and_duplicates() {
        let store = Arc::new(CountingStore {
            prefer_batch_reads: true,
            ..CountingStore::default()
        });
        let prolly = Prolly::new(store.clone(), Config::default());
        let unique_count = PARALLEL_NODE_DECODE_THRESHOLD + 4;
        let mut cids = Vec::new();

        for idx in 0..unique_count {
            let mut node = Node::new_leaf();
            node.keys.push(format!("k{idx:02}").into_bytes());
            node.vals.push(format!("v{idx:02}").into_bytes());
            cids.push(prolly.save(&node).unwrap());
        }

        let mut requested = Vec::with_capacity(unique_count + 2);
        requested.push(cids[3].clone());
        requested.extend(cids.iter().cloned());
        requested.push(cids[3].clone());
        prolly.clear_cache();

        let nodes = prolly
            .load_many_ordered_with_parallelism(&requested, 4)
            .unwrap();

        assert_eq!(nodes.len(), requested.len());
        assert!(Arc::ptr_eq(&nodes[0], nodes.last().unwrap()));
        assert!(Arc::ptr_eq(&nodes[0], &nodes[4]));
        for (idx, node) in nodes[1..=unique_count].iter().enumerate() {
            assert_eq!(node.keys[0], format!("k{idx:02}").into_bytes());
        }
        assert_eq!(prolly.cache_len(), unique_count);
        assert_eq!(store.batch_get_ordered_calls.load(Ordering::Relaxed), 4);
        assert!(
            store.max_batch_get_ordered_len.load(Ordering::Relaxed) <= unique_count.div_ceil(4),
            "wide parallel-decode misses should still use bounded ordered batches"
        );
    }

    #[test]
    fn collect_stats_batches_child_frontiers_for_batched_read_stores() {
        let store = Arc::new(CountingStore {
            prefer_batch_reads: true,
            ..CountingStore::default()
        });
        let prolly = Prolly::new(store.clone(), Config::default());
        let mut child_cids = Vec::new();

        for idx in 0..4 {
            let mut leaf = prolly.new_leaf_node();
            leaf.keys.push(format!("k{idx:02}").into_bytes());
            leaf.vals.push(format!("v{idx:02}").into_bytes());
            child_cids.push(prolly.save(&leaf).unwrap());
        }

        let mut root = prolly.new_internal_node(1);
        root.keys = (0..4)
            .map(|idx| format!("k{idx:02}").into_bytes())
            .collect();
        root.vals = child_cids.iter().map(|cid| cid.0.to_vec()).collect();
        let tree = Tree {
            root: Some(prolly.save(&root).unwrap()),
            config: Config::default(),
        };

        prolly.clear_cache();
        let stats = prolly.collect_stats(&tree).unwrap();

        assert_eq!(stats.num_nodes, 5);
        assert_eq!(stats.num_internal_nodes, 1);
        assert_eq!(stats.num_leaves, 4);
        assert_eq!(stats.total_key_value_pairs, 4);
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            0,
            "batched stats collection should hydrate frontiers through ordered batch reads"
        );
        assert_eq!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed),
            2,
            "stats should load the root frontier and then all leaf children as one child frontier"
        );
        assert_eq!(
            store.max_batch_get_ordered_len.load(Ordering::Relaxed),
            4,
            "the child frontier should be loaded as a single ordered batch"
        );
        assert_eq!(prolly.cache_len(), 5);
    }

    #[test]
    fn collect_stats_rejects_nodes_with_mismatched_values() {
        let store = Arc::new(CountingStore::default());
        let prolly = Prolly::new(store, Config::default());
        let mut child = prolly.new_leaf_node();
        child.keys.push(b"k".to_vec());
        child.vals.push(b"v".to_vec());
        let child_cid = prolly.save(&child).unwrap();

        let mut malformed = prolly.new_internal_node(1);
        malformed.keys = vec![b"a".to_vec(), b"m".to_vec()];
        malformed.vals = vec![child_cid.0.to_vec()];
        let tree = Tree {
            root: Some(prolly.save(&malformed).unwrap()),
            config: Config::default(),
        };
        prolly.clear_cache();

        let err = prolly.collect_stats(&tree).unwrap_err();

        assert!(
            matches!(err, Error::InvalidNode | Error::Deserialize(_)),
            "malformed stats roots should not be silently accepted: {err:?}"
        );
    }

    #[test]
    fn sorted_key_positions_keeps_already_sorted_inputs_in_place() {
        let keys = vec![b"a".to_vec(), b"b".to_vec(), b"b".to_vec(), b"c".to_vec()];

        let positions = sorted_key_positions(&keys);

        assert_eq!(positions, vec![0, 1, 2, 3]);
    }

    #[test]
    fn sorted_key_positions_sorts_unsorted_inputs_stably() {
        let keys = vec![b"c".to_vec(), b"a".to_vec(), b"b".to_vec(), b"a".to_vec()];

        let positions = sorted_key_positions(&keys);

        assert_eq!(positions, vec![1, 3, 2, 0]);
    }

    #[test]
    fn get_many_child_routing_keeps_singleton_positions_inline() {
        let child_cids = [
            Cid::from_bytes(b"child-0"),
            Cid::from_bytes(b"child-1"),
            Cid::from_bytes(b"child-2"),
        ];
        let mut node = Node::new_internal(1);
        node.keys = vec![b"a".to_vec(), b"d".to_vec(), b"g".to_vec()];
        node.vals = child_cids.iter().map(|cid| cid.0.to_vec()).collect();
        let keys = vec![b"a".to_vec(), b"d".to_vec(), b"g".to_vec()];
        let positions = InlinePositions::from_vec(vec![0, 1, 2]).unwrap();

        let frames = route_key_positions_to_children(&node, positions, &keys).unwrap();

        assert_eq!(frames.len(), 3);
        for (idx, frame) in frames.iter().enumerate() {
            assert_eq!(frame.cid, child_cids[idx]);
            assert_eq!(frame.positions.first, idx);
            assert_eq!(frame.positions.rest.capacity(), 0);
        }
    }

    #[test]
    fn lookup_child_index_uses_separator_floor() {
        let mut node = Node::new_internal(1);
        node.keys = vec![b"a".to_vec(), b"d".to_vec(), b"g".to_vec()];

        assert_eq!(child_index_for_lookup_key(&node, b"0"), 0);
        assert_eq!(child_index_for_lookup_key(&node, b"a"), 0);
        assert_eq!(child_index_for_lookup_key(&node, b"c"), 0);
        assert_eq!(child_index_for_lookup_key(&node, b"d"), 1);
        assert_eq!(child_index_for_lookup_key(&node, b"f"), 1);
        assert_eq!(child_index_for_lookup_key(&node, b"g"), 2);
        assert_eq!(child_index_for_lookup_key(&node, b"z"), 2);
    }

    #[test]
    fn get_many_child_routing_starts_at_first_target_child() {
        let child_cids = [
            Cid::from_bytes(b"child-0"),
            Cid::from_bytes(b"child-1"),
            Cid::from_bytes(b"child-2"),
            Cid::from_bytes(b"child-3"),
        ];
        let mut node = Node::new_internal(1);
        node.keys = vec![b"a".to_vec(), b"d".to_vec(), b"g".to_vec(), b"m".to_vec()];
        node.vals = child_cids.iter().map(|cid| cid.0.to_vec()).collect();
        let keys = vec![b"h".to_vec(), b"z".to_vec()];
        let positions = InlinePositions::from_vec(vec![0, 1]).unwrap();

        let frames = route_key_positions_to_children(&node, positions, &keys).unwrap();

        assert_eq!(frames.len(), 2);
        assert_eq!(frames[0].cid, child_cids[2]);
        assert_eq!(frames[0].positions.first, 0);
        assert_eq!(frames[1].cid, child_cids[3]);
        assert_eq!(frames[1].positions.first, 1);
    }

    #[test]
    fn get_many_boundary_routing_skips_empty_children_and_routes_separator_keys_right() {
        let child_cids = [
            Cid::from_bytes(b"child-0"),
            Cid::from_bytes(b"child-1"),
            Cid::from_bytes(b"child-2"),
            Cid::from_bytes(b"child-3"),
            Cid::from_bytes(b"child-4"),
            Cid::from_bytes(b"child-5"),
        ];
        let mut node = Node::new_internal(1);
        node.keys = [0, 10, 20, 30, 40, 50]
            .into_iter()
            .map(|idx| format!("k{idx:03}").into_bytes())
            .collect();
        node.vals = child_cids.iter().map(|cid| cid.0.to_vec()).collect();
        let lookup_keys = [0, 1, 2, 10, 11, 49, 50, 51]
            .into_iter()
            .map(|idx| format!("k{idx:03}").into_bytes())
            .collect::<Vec<_>>();
        let positions = InlinePositions::from_vec((0..lookup_keys.len()).collect()).unwrap();

        let frames =
            route_key_positions_to_children_by_boundary(&node, positions, &lookup_keys).unwrap();
        let routed = frames
            .into_iter()
            .map(|frame| (frame.cid, frame.positions.into_iter().collect::<Vec<_>>()))
            .collect::<Vec<_>>();

        assert_eq!(
            routed,
            vec![
                (child_cids[0].clone(), vec![0, 1, 2]),
                (child_cids[1].clone(), vec![3, 4]),
                (child_cids[4].clone(), vec![5]),
                (child_cids[5].clone(), vec![6, 7]),
            ]
        );
    }

    #[test]
    fn large_get_many_child_routing_keeps_clustered_positions_together() {
        let child_cids = (0..10)
            .map(|idx| Cid::from_bytes(format!("child-{idx}").as_bytes()))
            .collect::<Vec<_>>();
        let mut node = Node::new_internal(1);
        node.keys = (0..10)
            .map(|idx| format!("k{:03}", idx * 100).into_bytes())
            .collect();
        node.vals = child_cids.iter().map(|cid| cid.0.to_vec()).collect();
        let lookup_keys = (500..580)
            .map(|idx| format!("k{idx:03}").into_bytes())
            .collect::<Vec<_>>();
        let positions = InlinePositions::from_vec((0..lookup_keys.len()).collect()).unwrap();

        let frames = route_key_positions_to_children(&node, positions, &lookup_keys).unwrap();

        assert_eq!(frames.len(), 1);
        assert_eq!(frames[0].cid, child_cids[5]);
        assert_eq!(frames[0].positions.len(), lookup_keys.len());
        assert_eq!(frames[0].positions.first, 0);
        assert_eq!(
            frames[0].positions.rest.last(),
            Some(&(lookup_keys.len() - 1))
        );
    }

    #[test]
    fn get_many_preserves_input_order_duplicates_and_missing_keys() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();
        let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();
        let tree = prolly.put(&tree, b"c".to_vec(), b"3".to_vec()).unwrap();
        let keys = vec![
            b"c".to_vec(),
            b"missing".to_vec(),
            b"a".to_vec(),
            b"c".to_vec(),
        ];

        let values = prolly.get_many(&tree, &keys).unwrap();

        assert_eq!(
            values,
            vec![
                Some(b"3".to_vec()),
                None,
                Some(b"1".to_vec()),
                Some(b"3".to_vec()),
            ]
        );
    }

    #[test]
    fn clustered_get_many_uses_point_reads_for_singleton_frontiers_without_batched_read_preference()
    {
        let store = Arc::new(CountingStore::default());
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(4)
            .chunking_factor(u32::MAX)
            .build();

        let mut builder = builder::BatchBuilder::new(store.clone(), config.clone());
        for idx in 0..128 {
            builder.add(
                format!("k{idx:03}").into_bytes(),
                format!("v{idx:03}").into_bytes(),
            );
        }
        let tree = builder.build().unwrap();
        let prolly = Prolly::new(store.clone(), config);
        prolly.clear_cache();
        let batch_gets_before = store.batch_get_ordered_calls.load(Ordering::Relaxed);

        let values = prolly
            .get_many(
                &tree,
                &[b"k001".to_vec(), b"k002".to_vec(), b"k003".to_vec()],
            )
            .unwrap();

        assert_eq!(
            values,
            vec![
                Some(b"v001".to_vec()),
                Some(b"v002".to_vec()),
                Some(b"v003".to_vec())
            ]
        );
        assert_eq!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed),
            batch_gets_before,
            "clustered get_many should avoid one-key ordered batch reads at each level"
        );
        assert!(
            store.get_calls.load(Ordering::Relaxed) > 0,
            "clustered get_many should still hydrate the singleton path"
        );
    }

    #[test]
    fn get_many_rejects_leaf_with_mismatched_values() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let mut leaf = prolly.new_leaf_node();
        leaf.keys.push(b"a".to_vec());
        let tree = Tree {
            root: Some(prolly.save(&leaf).unwrap()),
            config: Config::default(),
        };

        let err = prolly.get_many(&tree, &[b"a".to_vec()]).unwrap_err();

        assert!(matches!(err, Error::InvalidNode));
    }

    #[test]
    fn get_rejects_leaf_with_mismatched_values() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let mut leaf = prolly.new_leaf_node();
        leaf.keys.push(b"a".to_vec());
        let tree = Tree {
            root: Some(prolly.save(&leaf).unwrap()),
            config: Config::default(),
        };

        let err = match prolly.get(&tree, b"a") {
            Ok(_) => panic!("malformed leaf should be rejected"),
            Err(err) => err,
        };

        assert!(matches!(err, Error::InvalidNode));
    }

    #[test]
    fn get_and_find_path_reject_internal_node_with_missing_child_cid() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let mut root = Node::new_internal(1);
        root.keys.push(b"a".to_vec());
        let tree = Tree {
            root: Some(prolly.save(&root).unwrap()),
            config: Config::default(),
        };

        let get_err = match prolly.get(&tree, b"a") {
            Ok(_) => panic!("malformed internal node should be rejected by get"),
            Err(err) => err,
        };
        let path_err = match prolly.find_path(&tree, b"a") {
            Ok(_) => panic!("malformed internal node should be rejected by find_path"),
            Err(err) => err,
        };

        assert!(matches!(get_err, Error::InvalidNode));
        assert!(matches!(path_err, Error::InvalidNode));
    }

    #[test]
    fn get_many_rejects_internal_node_with_missing_child_cid() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let mut root = Node::new_internal(1);
        root.keys.push(b"a".to_vec());
        let tree = Tree {
            root: Some(prolly.save(&root).unwrap()),
            config: Config::default(),
        };

        let err = prolly.get_many(&tree, &[b"a".to_vec()]).unwrap_err();

        assert!(matches!(err, Error::InvalidNode));
    }

    #[test]
    fn get_many_splits_wide_frontiers_for_batched_read_stores() {
        let store = Arc::new(CountingStore {
            prefer_batch_reads: true,
            ..CountingStore::default()
        });
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(4)
            .chunking_factor(u32::MAX)
            .build();
        let key_for = |idx: usize| format!("k{idx:04}").into_bytes();

        let mut builder = builder::BatchBuilder::new(store.clone(), config.clone());
        for idx in 0..4096 {
            builder.add(key_for(idx), format!("v{idx:04}").into_bytes());
        }
        let tree = builder.build().unwrap();
        let prolly = Prolly::new(store.clone(), config);
        let indices = (0..4096).step_by(8).rev().collect::<Vec<_>>();
        let keys = indices.iter().map(|idx| key_for(*idx)).collect::<Vec<_>>();

        prolly.clear_cache();
        let calls_before = store.batch_get_ordered_calls.load(Ordering::Relaxed);
        let values = prolly.get_many(&tree, &keys).unwrap();

        assert_eq!(values.len(), keys.len());
        for (idx, value) in values.into_iter().enumerate() {
            assert_eq!(value, Some(format!("v{:04}", indices[idx]).into_bytes()));
        }
        assert!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed)
                > calls_before + GET_MANY_PREFETCH_PARALLELISM,
            "wide get_many should split frontier reads into parallel ordered batches"
        );
        assert!(
            store.max_batch_get_ordered_len.load(Ordering::Relaxed) <= 64,
            "bounded parallel get_many should avoid one huge ordered batch for hundreds of misses"
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
    fn range_empty_half_open_bounds_skip_tree_seek() {
        let store = Arc::new(CountingStore::default());
        let prolly = Prolly::new(store.clone(), Config::default());
        let tree = prolly
            .put(&prolly.create(), b"k001".to_vec(), b"v001".to_vec())
            .unwrap();
        prolly.clear_cache();
        let get_calls_before = store.get_calls.load(Ordering::Relaxed);
        let batch_get_calls_before = store.batch_get_ordered_calls.load(Ordering::Relaxed);

        let results = prolly
            .range(&tree, b"k010", Some(b"k001"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert!(results.is_empty());
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            get_calls_before,
            "empty half-open ranges should not seek into the tree"
        );
        assert_eq!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed),
            batch_get_calls_before,
            "empty half-open ranges should not batch-load nodes"
        );
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
    fn range_rejects_leaf_with_mismatched_values() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let mut leaf = prolly.new_leaf_node();
        leaf.keys.push(b"a".to_vec());
        let tree = Tree {
            root: Some(prolly.save(&leaf).unwrap()),
            config: Config::default(),
        };

        let err = prolly.range(&tree, &[], None).unwrap().next().unwrap();

        assert!(matches!(err, Err(Error::InvalidNode)));
    }

    #[test]
    fn range_rejects_internal_node_with_missing_next_child_cid() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let mut first_leaf = prolly.new_leaf_node();
        first_leaf.keys.push(b"a".to_vec());
        first_leaf.vals.push(b"1".to_vec());
        let first_cid = prolly.save(&first_leaf).unwrap();

        let mut root = prolly.new_internal_node(1);
        root.keys = vec![b"a".to_vec(), b"m".to_vec()];
        root.vals = vec![first_cid.0.to_vec()];
        let tree = Tree {
            root: Some(prolly.save(&root).unwrap()),
            config: Config::default(),
        };

        let mut iter = prolly.range(&tree, &[], None).unwrap();
        assert_eq!(
            iter.next().unwrap().unwrap(),
            (b"a".to_vec(), b"1".to_vec())
        );
        let err = iter.next().unwrap();

        assert!(matches!(err, Err(Error::InvalidNode)));
    }

    #[test]
    fn range_end_bound_skips_loading_next_child_subtree() {
        let store = Arc::new(CountingStore::default());
        let prolly = Prolly::new(store.clone(), Config::default());

        let mut first = prolly.new_leaf_node();
        first.keys = vec![b"a".to_vec(), b"b".to_vec()];
        first.vals = vec![b"1".to_vec(), b"2".to_vec()];
        let first_cid = prolly.save(&first).unwrap();

        let mut second = prolly.new_leaf_node();
        second.keys = vec![b"c".to_vec(), b"d".to_vec()];
        second.vals = vec![b"3".to_vec(), b"4".to_vec()];
        let second_cid = prolly.save(&second).unwrap();

        let mut third = prolly.new_leaf_node();
        third.keys = vec![b"e".to_vec(), b"f".to_vec()];
        third.vals = vec![b"5".to_vec(), b"6".to_vec()];
        let third_cid = prolly.save(&third).unwrap();

        let mut root = prolly.new_internal_node(1);
        root.keys = vec![b"a".to_vec(), b"c".to_vec(), b"e".to_vec()];
        root.vals = vec![
            first_cid.0.to_vec(),
            second_cid.0.to_vec(),
            third_cid.0.to_vec(),
        ];
        let tree = Tree {
            root: Some(prolly.save(&root).unwrap()),
            config: Config::default(),
        };

        prolly.clear_cache();
        let gets_before = store.get_calls.load(Ordering::Relaxed);
        let results = prolly
            .range(&tree, &[], Some(b"e"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(
            results,
            vec![
                (b"a".to_vec(), b"1".to_vec()),
                (b"b".to_vec(), b"2".to_vec()),
                (b"c".to_vec(), b"3".to_vec()),
                (b"d".to_vec(), b"4".to_vec()),
            ]
        );
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed) - gets_before,
            3,
            "bounded range should load root and in-range leaves, not the first leaf at the exclusive end"
        );
    }

    #[test]
    fn range_batches_in_range_sibling_hydration_for_batched_read_stores() {
        let store = Arc::new(CountingStore {
            prefer_batch_reads: true,
            ..CountingStore::default()
        });
        let prolly = Prolly::new(store.clone(), Config::default());

        let mut child_cids = Vec::new();
        let mut expected = Vec::new();
        for leaf_idx in 0..5 {
            let mut leaf = prolly.new_leaf_node();
            for entry_idx in 0..2 {
                let idx = leaf_idx * 2 + entry_idx;
                leaf.keys.push(format!("k{idx:02}").into_bytes());
                leaf.vals.push(format!("v{idx:02}").into_bytes());
                if idx < 6 {
                    expected.push((
                        format!("k{idx:02}").into_bytes(),
                        format!("v{idx:02}").into_bytes(),
                    ));
                }
            }
            child_cids.push(prolly.save(&leaf).unwrap());
        }

        let mut root = prolly.new_internal_node(1);
        root.keys = (0..5)
            .map(|leaf_idx| format!("k{:02}", leaf_idx * 2).into_bytes())
            .collect();
        root.vals = child_cids.iter().map(|cid| cid.0.to_vec()).collect();
        let tree = Tree {
            root: Some(prolly.save(&root).unwrap()),
            config: Config::default(),
        };

        prolly.clear_cache();
        let gets_before = store.get_calls.load(Ordering::Relaxed);
        let ordered_gets_before = store.batch_get_ordered_calls.load(Ordering::Relaxed);
        let results = prolly
            .range(&tree, &[], Some(b"k06"))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(results, expected);
        assert!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed) > ordered_gets_before,
            "batched-read range scans should hydrate upcoming in-range siblings together"
        );
        assert_eq!(
            store.max_batch_get_ordered_len.load(Ordering::Relaxed),
            2,
            "prefetch should include the two remaining in-range leaves but stop before the exclusive end"
        );
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed) - gets_before,
            2,
            "range should only point-read the root and initial seek leaf"
        );
        assert_eq!(
            prolly.cache_len(),
            4,
            "cache should contain root plus the three in-range leaves, not the leaf at the exclusive end"
        );
    }

    #[test]
    fn range_reuses_cached_nodes_on_repeated_scan() {
        let store = Arc::new(CountingStore::default());
        let prolly = Prolly::new(store.clone(), Config::default());

        let mut first = prolly.new_leaf_node();
        first.keys = vec![b"a".to_vec(), b"b".to_vec()];
        first.vals = vec![b"1".to_vec(), b"2".to_vec()];
        let first_cid = prolly.save(&first).unwrap();

        let mut second = prolly.new_leaf_node();
        second.keys = vec![b"c".to_vec(), b"d".to_vec()];
        second.vals = vec![b"3".to_vec(), b"4".to_vec()];
        let second_cid = prolly.save(&second).unwrap();

        let mut root = prolly.new_internal_node(1);
        root.keys = vec![b"a".to_vec(), b"c".to_vec()];
        root.vals = vec![first_cid.0.to_vec(), second_cid.0.to_vec()];
        let tree = Tree {
            root: Some(prolly.save(&root).unwrap()),
            config: Config::default(),
        };

        prolly.clear_cache();
        let first_results = prolly
            .range(&tree, &[], None)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        let gets_after_first = store.get_calls.load(Ordering::Relaxed);
        let second_results = prolly
            .range(&tree, &[], None)
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(first_results, second_results);
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            gets_after_first,
            "second range scan should reuse cached Arc<Node> path and child leaves"
        );
    }

    #[test]
    fn range_cursor_end_bound_skips_loading_next_child_subtree() {
        let store = Arc::new(CountingStore::default());
        let prolly = Prolly::new(store.clone(), Config::default());

        let mut first = prolly.new_leaf_node();
        first.keys = vec![b"a".to_vec(), b"b".to_vec()];
        first.vals = vec![b"1".to_vec(), b"2".to_vec()];
        let first_cid = prolly.save(&first).unwrap();

        let mut second = prolly.new_leaf_node();
        second.keys = vec![b"c".to_vec(), b"d".to_vec()];
        second.vals = vec![b"3".to_vec(), b"4".to_vec()];
        let second_cid = prolly.save(&second).unwrap();

        let mut third = prolly.new_leaf_node();
        third.keys = vec![b"e".to_vec(), b"f".to_vec()];
        third.vals = vec![b"5".to_vec(), b"6".to_vec()];
        let third_cid = prolly.save(&third).unwrap();

        let mut root = prolly.new_internal_node(1);
        root.keys = vec![b"a".to_vec(), b"c".to_vec(), b"e".to_vec()];
        root.vals = vec![
            first_cid.0.to_vec(),
            second_cid.0.to_vec(),
            third_cid.0.to_vec(),
        ];
        let tree = Tree {
            root: Some(prolly.save(&root).unwrap()),
            config: Config::default(),
        };

        let gets_before = store.get_calls.load(Ordering::Relaxed);
        let results = prolly
            .range_cursor(&tree, &[], Some(b"e"))
            .unwrap()
            .collect::<Vec<_>>();

        assert_eq!(
            results,
            vec![
                (b"a".to_vec(), b"1".to_vec()),
                (b"b".to_vec(), b"2".to_vec()),
                (b"c".to_vec(), b"3".to_vec()),
                (b"d".to_vec(), b"4".to_vec()),
            ]
        );
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed) - gets_before,
            3,
            "bounded cursor range should load root and in-range leaves, not the first leaf at the exclusive end"
        );
    }

    #[test]
    fn range_cursor_empty_half_open_bounds_skip_tree_seek() {
        let store = Arc::new(CountingStore::default());
        let prolly = Prolly::new(store.clone(), Config::default());
        let tree = prolly
            .put(&prolly.create(), b"k001".to_vec(), b"v001".to_vec())
            .unwrap();
        let get_calls_before = store.get_calls.load(Ordering::Relaxed);

        let results = prolly
            .range_cursor(&tree, b"k010", Some(b"k001"))
            .unwrap()
            .collect::<Vec<_>>();

        assert!(results.is_empty());
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            get_calls_before,
            "empty cursor ranges should not load the root node"
        );
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
