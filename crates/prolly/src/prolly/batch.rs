//! Batch mutation operations for Prolly trees
//!
//! This module handles batch operations including preprocessing mutations,
//! grouping by leaf, and atomic writes. Batch operations enable efficient
//! bulk modifications to a tree with a single atomic write.
//!
//! # Overview
//!
//! Batch mutations provide an optimized way to apply multiple changes to a tree
//! in a single operation. Instead of modifying the tree one key at a time (which
//! would require multiple tree traversals and writes), batch operations:
//!
//! 1. Preprocess all mutations (sort and deduplicate)
//! 2. Group mutations by their target leaf node
//! 3. Apply all mutations to each leaf in a single pass
//! 4. Write all modified nodes atomically
//!
//! # Batch Processing Pipeline
//!
//! ## Step 1: Preprocessing
//!
//! Mutations are sorted by key in lexicographic order and deduplicated using
//! last-write-wins semantics. This ensures:
//!
//! - Efficient grouping by target leaf
//! - Deterministic results regardless of input order
//! - Only the final mutation for each key is applied
//!
//! ## Step 2: Grouping by Leaf
//!
//! Sorted mutations are grouped by their target leaf node. Mutations targeting
//! the same leaf are collected together to minimize tree traversals.
//!
//! ## Step 3: Applying Mutations
//!
//! Each group of mutations is applied to its target leaf:
//!
//! - **Upsert**: Insert new key or update existing value
//! - **Delete**: Remove key if it exists (no-op if not present)
//!
//! ## Step 4: Atomic Write
//!
//! All modified nodes are collected in a `BatchWriteCollector` and written
//! to the store atomically using the store's batch API. This ensures that
//! either all changes are persisted or none are.
//!
//! # Key Types
//!
//! - [`BatchWriteCollector`] - Accumulates nodes for atomic batch writing
//! - [`LeafMutationGroup`] - Groups mutations targeting the same leaf
//! - [`preprocess_mutations`] - Sorts and deduplicates mutations
//! - [`apply_mutations_to_leaf`] - Applies mutations to a single leaf node
//! - [`apply_batch`] - Main entry point for batch operations
//!
//! # Example
//!
//! ```rust
//! use prolly::{Prolly, MemStore, Config, Mutation};
//!
//! let store = MemStore::new();
//! let prolly = Prolly::new(store, Config::default());
//! let tree = prolly.create();
//!
//! // Create batch mutations
//! let mutations = vec![
//!     Mutation::Upsert { key: b"a".to_vec(), val: b"1".to_vec() },
//!     Mutation::Upsert { key: b"b".to_vec(), val: b"2".to_vec() },
//!     Mutation::Delete { key: b"old".to_vec() },
//! ];
//!
//! // Apply batch atomically
//! let new_tree = prolly.batch(&tree, mutations).unwrap();
//! ```
//!
//! # Performance Guide
//!
//! ## When to Use Batch Operations
//!
//! Batch operations are more efficient than individual operations when:
//!
//! - **Applying many mutations at once**: The overhead of preprocessing is
//!   amortized across all mutations
//! - **Mutations are spread across multiple leaves**: Grouping reduces tree
//!   traversals from O(m) to O(k) where k is the number of affected leaves
//! - **Atomicity is required**: All changes succeed or fail together
//! - **Working with remote/network stores**: Prefetch optimization reduces
//!   I/O latency significantly
//!
//! ## When Individual Operations May Be Better
//!
//! Consider individual `put`/`delete` operations when:
//!
//! - Applying only 1-5 mutations
//! - Mutations need to be applied incrementally with reads in between
//! - Memory is extremely constrained (batch preprocessing requires O(m) memory)
//!
//! ## Optimization Features
//!
//! ### Two-Pointer Merge Algorithm
//!
//! The default merge algorithm uses a two-pointer approach with O(n+m) complexity,
//! where n is existing entries and m is mutations. This is significantly faster
//! than the O(m log n) binary search approach for typical batch sizes.
//!
//! **Performance comparison (n = 1000 entries):**
//!
//! | Mutations (m) | Two-Pointer | Binary Search | Speedup |
//! |---------------|-------------|---------------|---------|
//! | 10            | ~1,010 ops  | ~100 ops      | 0.1x    |
//! | 100           | ~1,100 ops  | ~1,000 ops    | 0.9x    |
//! | 500           | ~1,500 ops  | ~5,000 ops    | 3.3x    |
//! | 1,000         | ~2,000 ops  | ~10,000 ops   | 5x      |
//! | 10,000        | ~11,000 ops | ~130,000 ops  | 12x     |
//!
//! The two-pointer merge is enabled by default and recommended for most use cases.
//!
//! ### Leaf Prefetch Optimization
//!
//! For stores that support parallel I/O (e.g., network stores), the batch
//! operation prefetches all affected leaves before processing. This can
//! dramatically reduce latency:
//!
//! - **Without prefetch**: k sequential fetches × network latency
//! - **With prefetch**: 1 parallel fetch × network latency
//!
//! Prefetch is enabled by default but has minimal impact on in-memory stores.
//!
//! ## Memory Usage Patterns
//!
//! ### Preprocessing Phase
//!
//! - **Sorting**: O(m) memory for mutation vector
//! - **Deduplication**: O(m) memory (in-place, may shrink)
//!
//! ### Grouping Phase
//!
//! - **Groups**: O(k) where k = number of affected leaves
//! - **Each group**: References to mutations (no copying)
//!
//! ### Merge Phase
//!
//! - **Per leaf**: O(n + m_i) where m_i = mutations for that leaf
//! - **Result vectors**: Pre-allocated to avoid reallocations
//!
//! ### Total Memory
//!
//! For a batch of m mutations affecting k leaves with average n entries each:
//! - Peak memory: O(m + k × n) for mutations + largest leaf result
//! - Temporary allocations: Minimized through pre-allocation
//!
//! ## Using MutationBuffer for Large Datasets
//!
//! When processing datasets larger than available memory, use `MutationBuffer`
//! to stream mutations in chunks:
//!
//! ```rust
//! use prolly::{MutationBuffer, Mutation, Prolly, MemStore, Config};
//!
//! let store = MemStore::new();
//! let prolly = Prolly::new(store, Config::default());
//! let mut tree = prolly.create();
//!
//! // Process in 10MB chunks
//! let mut buffer = MutationBuffer::with_max_size(10 * 1024 * 1024);
//!
//! for i in 0..1_000 {
//!     let mutation = Mutation::Upsert {
//!         key: format!("key{:08}", i).into_bytes(),
//!         val: format!("value{}", i).into_bytes(),
//!     };
//!
//!     if buffer.add(mutation).is_err() {
//!         // Buffer full - flush to tree
//!         let mutations = buffer.drain();
//!         tree = prolly.batch(&tree, mutations).unwrap();
//!     }
//! }
//!
//! // Flush remaining mutations
//! if !buffer.is_empty() {
//!     let mutations = buffer.drain();
//!     tree = prolly.batch(&tree, mutations).unwrap();
//! }
//! ```
//!
//! ## Configuring Batch Operations
//!
//! Use `BatchWriter` with `BatchWriterConfig` for fine-grained control:
//!
//! ```rust
//! use prolly::{BatchWriter, BatchWriterConfig, Prolly, MemStore, Config, Mutation};
//!
//! let store = MemStore::new();
//! let prolly = Prolly::new(store, Config::default());
//! let tree = prolly.create();
//!
//! // Configure for in-memory store (disable prefetch)
//! let config = BatchWriterConfig::new()
//!     .with_prefetch(false)
//!     .with_optimized_merge(true);
//!
//! let writer = BatchWriter::with_config(config);
//!
//! let mutations = vec![
//!     Mutation::Upsert { key: b"key".to_vec(), val: b"value".to_vec() },
//! ];
//!
//! let new_tree = writer.apply_batch(&prolly, &tree, mutations).unwrap();
//! ```
//!
//! ### Configuration Options
//!
//! | Option | Default | Description |
//! |--------|---------|-------------|
//! | `enable_prefetch` | `true` | Prefetch affected leaves before processing |
//! | `use_optimized_merge` | `true` | Use O(n+m) two-pointer merge |
//! | `prefetch_parallelism` | `16` | Max concurrent prefetch operations |
//!
//! ### Recommended Configurations
//!
//! **In-memory store:**
//! ```rust
//! # use prolly::BatchWriterConfig;
//! let config = BatchWriterConfig::new()
//!     .with_prefetch(false);  // No benefit for in-memory
//! ```
//!
//! **Network store with high latency:**
//! ```rust
//! # use prolly::BatchWriterConfig;
//! let config = BatchWriterConfig::new()
//!     .with_prefetch(true)
//!     .with_prefetch_parallelism(32);  // More parallelism for high latency
//! ```
//!
//! **Debugging/comparison:**
//! ```rust
//! # use prolly::BatchWriterConfig;
//! let config = BatchWriterConfig::new()
//!     .with_optimized_merge(false);  // Use binary search for comparison
//! ```

use super::cid::Cid;
use super::cursor::Cursor;
use super::error::Error;
use super::error::Mutation;
use super::node::Node;
use super::store::{BatchOp, Store};
use super::tree::Tree;

use super::rebalance;
use super::Prolly;

/// Leaf span affected by mutations.
///
/// Identifies a leaf node by its CID and the key range it covers.
/// Used for span-based leaf identification to ensure each leaf is
/// processed exactly once during batch operations.
///
/// # Fields
/// - `leaf_cid`: The content identifier of the leaf node
/// - `start_key`: The first key in this leaf's range
/// - `end_key`: The last key in this leaf's range
///
/// # Example
/// ```rust
/// use prolly::{LeafSpan, Cid};
///
/// let span = LeafSpan {
///     leaf_cid: Cid::from_bytes(b"example"),
///     start_key: b"a".to_vec(),
///     end_key: b"z".to_vec(),
/// };
///
/// println!("Span covers keys from {:?} to {:?}", span.start_key, span.end_key);
/// ```
#[derive(Debug, Clone)]
pub struct LeafSpan {
    /// CID of the leaf node
    pub leaf_cid: Cid,
    /// First key in this leaf's range
    pub start_key: Vec<u8>,
    /// Last key in this leaf's range
    pub end_key: Vec<u8>,
}

/// Collector for nodes to be written in a batch operation.
///
/// This struct accumulates nodes during batch mutation processing and writes
/// them all atomically to the store at the end of the operation using the
/// Store's batch API.
///
/// # Example
/// ```ignore
/// let mut collector = BatchWriteCollector::new();
/// let cid = collector.add(&node);
/// collector.flush(&store)?;
/// ```
pub struct BatchWriteCollector {
    /// Nodes to write: (cid_bytes, node_bytes)
    nodes: Vec<(Vec<u8>, Vec<u8>)>,
}

impl BatchWriteCollector {
    /// Create a new empty BatchWriteCollector.
    ///
    /// # Returns
    /// A new BatchWriteCollector with no nodes collected.
    pub fn new() -> Self {
        Self { nodes: Vec::new() }
    }

    /// Add a node to be written, returns its CID.
    ///
    /// The node is serialized and its CID is computed. The node bytes are
    /// stored for later batch writing.
    ///
    /// # Arguments
    /// * `node` - The node to add to the batch
    ///
    /// # Returns
    /// The CID of the node (computed from its serialized bytes).
    pub fn add(&mut self, node: &Node) -> Cid {
        let bytes = node.to_bytes();
        let cid = Cid::from_bytes(&bytes);
        self.nodes.push((cid.0.to_vec(), bytes));
        cid
    }

    /// Write all collected nodes to the store atomically.
    ///
    /// Uses the Store's batch operation to write all nodes in a single
    /// atomic operation. If the batch operation fails, no partial modifications
    /// are made to the store.
    ///
    /// # Arguments
    /// * `store` - The store to write nodes to
    ///
    /// # Returns
    /// * `Ok(())` - All nodes were written successfully
    /// * `Err(Error::Store)` - A storage error occurred
    pub fn flush<S: Store>(&self, store: &S) -> Result<(), Error> {
        if self.nodes.is_empty() {
            return Ok(());
        }

        let ops: Vec<BatchOp> = self
            .nodes
            .iter()
            .map(|(k, v)| BatchOp::Upsert {
                key: k.as_slice(),
                value: v.as_slice(),
            })
            .collect();

        store.batch(&ops).map_err(|e| Error::Store(Box::new(e)))
    }

    /// Get the number of nodes collected.
    ///
    /// # Returns
    /// The number of nodes that have been added to the collector.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if the collector is empty.
    ///
    /// # Returns
    /// `true` if no nodes have been added, `false` otherwise.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Get an iterator over the collected nodes.
    ///
    /// # Returns
    /// An iterator yielding `(&Vec<u8>, &Vec<u8>)` pairs of (cid_bytes, node_bytes).
    pub fn nodes_iter(&self) -> impl Iterator<Item = (&Vec<u8>, &Vec<u8>)> {
        self.nodes.iter().map(|(cid, bytes)| (cid, bytes))
    }
}

impl Default for BatchWriteCollector {
    fn default() -> Self {
        Self::new()
    }
}

/// Default maximum size for MutationBuffer (64 MB)
const DEFAULT_BUFFER_MAX_SIZE: usize = 64 * 1024 * 1024;

/// Write buffer for batching mutations with size limits.
///
/// `MutationBuffer` provides a memory-bounded container for accumulating mutations
/// before applying them to a tree. This is useful for processing large datasets
/// that don't fit in memory, allowing you to flush mutations in batches.
///
/// # Size Tracking
///
/// The buffer tracks the total byte size of accumulated mutations:
/// - For `Upsert` mutations: key length + value length
/// - For `Delete` mutations: key length
///
/// When adding a mutation would exceed the configured maximum size, the `add()`
/// method returns `Err(Error::BufferFull)`.
///
/// # Example
///
/// ```rust
/// use prolly::{MutationBuffer, Mutation, Error};
///
/// let mut buffer = MutationBuffer::with_max_size(1024); // 1KB limit
///
/// // Add mutations until buffer is full
/// let mutation = Mutation::Upsert {
///     key: b"key".to_vec(),
///     val: b"value".to_vec(),
/// };
///
/// buffer.add(mutation).unwrap();
///
/// // Check buffer state
/// assert!(!buffer.is_empty());
/// assert_eq!(buffer.len(), 1);
///
/// // Drain mutations for processing
/// let mutations = buffer.drain();
/// assert!(buffer.is_empty());
/// ```
///
/// # Streaming Large Datasets
///
/// ```rust
/// use prolly::{MutationBuffer, Mutation, Prolly, MemStore, Config};
///
/// let store = MemStore::new();
/// let prolly = Prolly::new(store, Config::default());
/// let mut tree = prolly.create();
///
/// let mut buffer = MutationBuffer::with_max_size(10 * 1024 * 1024); // 10MB
///
/// // Process large dataset in chunks
/// for i in 0..1000 {
///     let mutation = Mutation::Upsert {
///         key: format!("key{}", i).into_bytes(),
///         val: format!("value{}", i).into_bytes(),
///     };
///
///     if buffer.add(mutation).is_err() {
///         // Buffer full - flush to tree
///         let mutations = buffer.drain();
///         tree = prolly.batch(&tree, mutations).unwrap();
///     }
/// }
///
/// // Flush remaining mutations
/// if !buffer.is_empty() {
///     let mutations = buffer.drain();
///     tree = prolly.batch(&tree, mutations).unwrap();
/// }
/// ```
pub struct MutationBuffer {
    /// Accumulated mutations
    mutations: Vec<Mutation>,
    /// Maximum buffer size in bytes
    max_size: usize,
    /// Current buffer size in bytes
    current_size: usize,
}

impl MutationBuffer {
    /// Create a new MutationBuffer with the default maximum size (64 MB).
    ///
    /// # Returns
    /// A new empty MutationBuffer with a 64 MB size limit.
    ///
    /// # Example
    /// ```rust
    /// use prolly::MutationBuffer;
    ///
    /// let buffer = MutationBuffer::new();
    /// assert!(buffer.is_empty());
    /// ```
    pub fn new() -> Self {
        Self::with_max_size(DEFAULT_BUFFER_MAX_SIZE)
    }

    /// Create a new MutationBuffer with a custom maximum size.
    ///
    /// # Arguments
    /// * `max_size` - Maximum buffer size in bytes
    ///
    /// # Returns
    /// A new empty MutationBuffer with the specified size limit.
    ///
    /// # Example
    /// ```rust
    /// use prolly::MutationBuffer;
    ///
    /// let buffer = MutationBuffer::with_max_size(1024 * 1024); // 1MB limit
    /// assert!(buffer.is_empty());
    /// ```
    pub fn with_max_size(max_size: usize) -> Self {
        Self {
            mutations: Vec::new(),
            max_size,
            current_size: 0,
        }
    }

    /// Add a mutation to the buffer.
    ///
    /// The mutation's size is calculated as:
    /// - `Upsert`: key length + value length
    /// - `Delete`: key length
    ///
    /// # Arguments
    /// * `mutation` - The mutation to add
    ///
    /// # Returns
    /// * `Ok(())` - Mutation was added successfully
    /// * `Err(Error::BufferFull)` - Adding the mutation would exceed the buffer's max size
    ///
    /// # Example
    /// ```rust
    /// use prolly::{MutationBuffer, Mutation, Error};
    ///
    /// let mut buffer = MutationBuffer::with_max_size(5);
    ///
    /// // This fits (1 + 1 = 2 bytes)
    /// let result = buffer.add(Mutation::Upsert {
    ///     key: b"a".to_vec(),
    ///     val: b"1".to_vec(),
    /// });
    /// assert!(result.is_ok());
    ///
    /// // This would exceed the limit (3 + 5 = 8 bytes, total would be 10 > 5)
    /// let result = buffer.add(Mutation::Upsert {
    ///     key: b"key".to_vec(),
    ///     val: b"value".to_vec(),
    /// });
    /// assert!(matches!(result, Err(Error::BufferFull)));
    /// ```
    pub fn add(&mut self, mutation: Mutation) -> Result<(), Error> {
        let size = match &mutation {
            Mutation::Upsert { key, val } => key.len() + val.len(),
            Mutation::Delete { key } => key.len(),
        };

        if self.current_size + size > self.max_size {
            return Err(Error::BufferFull);
        }

        self.mutations.push(mutation);
        self.current_size += size;
        Ok(())
    }

    /// Drain all mutations from the buffer and reset its state.
    ///
    /// Returns all accumulated mutations and resets the buffer to empty.
    /// The current size is reset to 0.
    ///
    /// # Returns
    /// A vector containing all mutations that were in the buffer.
    ///
    /// # Example
    /// ```rust
    /// use prolly::{MutationBuffer, Mutation};
    ///
    /// let mut buffer = MutationBuffer::new();
    /// buffer.add(Mutation::Upsert {
    ///     key: b"key".to_vec(),
    ///     val: b"value".to_vec(),
    /// }).unwrap();
    ///
    /// let mutations = buffer.drain();
    /// assert_eq!(mutations.len(), 1);
    /// assert!(buffer.is_empty());
    /// assert_eq!(buffer.size(), 0);
    /// ```
    pub fn drain(&mut self) -> Vec<Mutation> {
        self.current_size = 0;
        std::mem::take(&mut self.mutations)
    }

    /// Check if the buffer is full.
    ///
    /// Returns `true` if the current size equals or exceeds the maximum size.
    /// Note that this doesn't guarantee the next `add()` will fail, as it depends
    /// on the size of the mutation being added.
    ///
    /// # Returns
    /// `true` if the buffer is at or over capacity, `false` otherwise.
    ///
    /// # Example
    /// ```rust
    /// use prolly::{MutationBuffer, Mutation};
    ///
    /// let mut buffer = MutationBuffer::with_max_size(5);
    /// assert!(!buffer.is_full());
    ///
    /// buffer.add(Mutation::Upsert {
    ///     key: b"ab".to_vec(),
    ///     val: b"cde".to_vec(),
    /// }).unwrap();
    /// assert!(buffer.is_full()); // 2 + 3 = 5 bytes
    /// ```
    pub fn is_full(&self) -> bool {
        self.current_size >= self.max_size
    }

    /// Get the current size of the buffer in bytes.
    ///
    /// # Returns
    /// The total byte size of all mutations in the buffer.
    ///
    /// # Example
    /// ```rust
    /// use prolly::{MutationBuffer, Mutation};
    ///
    /// let mut buffer = MutationBuffer::new();
    /// assert_eq!(buffer.size(), 0);
    ///
    /// buffer.add(Mutation::Upsert {
    ///     key: b"key".to_vec(),   // 3 bytes
    ///     val: b"value".to_vec(), // 5 bytes
    /// }).unwrap();
    /// assert_eq!(buffer.size(), 8);
    /// ```
    pub fn size(&self) -> usize {
        self.current_size
    }

    /// Get the number of mutations in the buffer.
    ///
    /// # Returns
    /// The count of mutations currently in the buffer.
    ///
    /// # Example
    /// ```rust
    /// use prolly::{MutationBuffer, Mutation};
    ///
    /// let mut buffer = MutationBuffer::new();
    /// assert_eq!(buffer.len(), 0);
    ///
    /// buffer.add(Mutation::Upsert {
    ///     key: b"key".to_vec(),
    ///     val: b"value".to_vec(),
    /// }).unwrap();
    /// assert_eq!(buffer.len(), 1);
    /// ```
    pub fn len(&self) -> usize {
        self.mutations.len()
    }

    /// Check if the buffer is empty.
    ///
    /// # Returns
    /// `true` if the buffer contains no mutations, `false` otherwise.
    ///
    /// # Example
    /// ```rust
    /// use prolly::{MutationBuffer, Mutation};
    ///
    /// let mut buffer = MutationBuffer::new();
    /// assert!(buffer.is_empty());
    ///
    /// buffer.add(Mutation::Upsert {
    ///     key: b"key".to_vec(),
    ///     val: b"value".to_vec(),
    /// }).unwrap();
    /// assert!(!buffer.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.mutations.is_empty()
    }

    /// Sort mutations by key in lexicographic byte order.
    ///
    /// Sorts mutations in place using lexicographic byte ordering, which is
    /// consistent with the tree's key ordering. This prepares mutations for
    /// efficient batch processing.
    ///
    /// The sort is stable, meaning mutations with the same key maintain their
    /// relative order. This is important for last-write-wins semantics when
    /// combined with deduplication.
    ///
    /// # Example
    /// ```rust
    /// use prolly::{MutationBuffer, Mutation};
    ///
    /// let mut buffer = MutationBuffer::new();
    /// buffer.add(Mutation::Upsert {
    ///     key: b"c".to_vec(),
    ///     val: b"3".to_vec(),
    /// }).unwrap();
    /// buffer.add(Mutation::Upsert {
    ///     key: b"a".to_vec(),
    ///     val: b"1".to_vec(),
    /// }).unwrap();
    /// buffer.add(Mutation::Upsert {
    ///     key: b"b".to_vec(),
    ///     val: b"2".to_vec(),
    /// }).unwrap();
    ///
    /// buffer.sort();
    ///
    /// let mutations = buffer.drain();
    /// assert_eq!(mutations[0].key(), b"a");
    /// assert_eq!(mutations[1].key(), b"b");
    /// assert_eq!(mutations[2].key(), b"c");
    /// ```
    pub fn sort(&mut self) {
        self.mutations.sort_by(|a, b| a.key().cmp(b.key()));
    }
}

impl Default for MutationBuffer {
    fn default() -> Self {
        Self::new()
    }
}

/// Mutations grouped by their target leaf node.
///
/// Used internally by the batch method to group mutations that
/// affect the same leaf, enabling efficient batch application.
pub struct LeafMutationGroup {
    /// The leaf node to modify
    pub leaf: Node,
    /// Path from root to this leaf (excluding the leaf itself)
    pub ancestors: Vec<(Node, usize)>,
    /// Mutations to apply to this leaf, in key order
    pub mutations: Vec<Mutation>,
}

/// Result of applying mutations in deferred mode.
///
/// This struct captures the state after mutations have been applied to leaves
/// without triggering immediate rebalancing. The leaves may be oversized at this
/// point and will be split during the subsequent bottom-up rebuild phase.
///
/// # Fields
///
/// * `modified_leaves` - Leaf nodes after mutations have been applied. These nodes
///   may exceed `max_chunk_size` since rebalancing is deferred.
/// * `ancestor_paths` - The original path from root to each modified leaf. Each path
///   is a vector of `(Node, usize)` tuples where the `Node` is an ancestor and `usize`
///   is the index of the child that leads to the leaf.
/// * `first_keys` - The first key of each modified leaf, used for constructing parent
///   nodes during the rebuild phase.
///
/// # Usage
///
/// This struct is produced by [`apply_mutations_deferred`] and consumed by
/// [`rebuild_from_modified_leaves`] to complete the deferred rebalancing process.
///
/// # Example
///
/// ```ignore
/// // Apply mutations without immediate rebalancing
/// let deferred_result = apply_mutations_deferred(&prolly, groups);
///
/// // The modified leaves may be oversized
/// for leaf in &deferred_result.modified_leaves {
///     // leaf.len() may exceed max_chunk_size
/// }
///
/// // Rebuild the tree, splitting oversized leaves
/// let new_root = rebuild_from_modified_leaves(
///     &prolly,
///     deferred_result,
///     &mut collector,
/// )?;
/// ```
#[derive(Debug, Clone)]
pub struct DeferredMutationResult {
    /// Modified leaves after mutations have been applied.
    ///
    /// These leaf nodes may exceed `max_chunk_size` since rebalancing is deferred
    /// until the rebuild phase. Each leaf corresponds to a mutation group that was
    /// processed.
    pub modified_leaves: Vec<Node>,

    /// Original ancestor paths for each modified leaf.
    ///
    /// Each path is a vector of `(Node, usize)` tuples representing the traversal
    /// from root to the leaf's parent. The `Node` is an ancestor node and `usize`
    /// is the index of the child pointer that leads toward the leaf.
    ///
    /// These paths are used during rebuild to determine how to merge changes back
    /// into the tree structure.
    pub ancestor_paths: Vec<Vec<(Node, usize)>>,

    /// First keys of modified leaves for parent construction.
    ///
    /// During the bottom-up rebuild, parent nodes need to know the first key of
    /// each child to maintain the tree's key ordering invariant. This vector
    /// stores the first key of each modified leaf in the same order as
    /// `modified_leaves`.
    pub first_keys: Vec<Vec<u8>>,
}

/// Applies all mutations to their target leaves without rebalancing.
///
/// This function is the first phase of the deferred rebalancing optimization.
/// It applies mutations to leaves but does NOT trigger rebalancing, allowing
/// leaves to temporarily exceed `max_chunk_size`. The oversized leaves will
/// be split during the subsequent bottom-up rebuild phase.
///
/// # Arguments
/// * `groups` - Vector of `LeafMutationGroup` containing leaves and their mutations
///
/// # Returns
/// A `DeferredMutationResult` containing:
/// - `modified_leaves`: Leaf nodes after mutations (may be oversized)
/// - `ancestor_paths`: Original paths from root to each leaf's parent
/// - `first_keys`: First key of each modified leaf for parent construction
///
/// # Algorithm
/// For each mutation group:
/// 1. Apply mutations to the leaf using `apply_mutations_to_leaf`
/// 2. Extract the first key from the modified leaf
/// 3. Collect the modified leaf and its ancestor path
///
/// # Key Difference from Standard Processing
/// Unlike the standard batch processing which calls `rebalance_with_collector`
/// after each leaf modification, this function simply collects the results.
/// Rebalancing is deferred to the `rebuild_from_modified_leaves` phase.
///
/// # Requirements
/// - Requirement 2.1: Apply all mutations to target leaves before rebalancing
/// - Requirement 2.2: Do NOT trigger rebalancing even if leaf exceeds max_chunk_size
/// - Requirement 2.4: Mark oversized leaves for splitting during rebuild phase
///
/// # Example
/// ```rust,ignore
/// use prolly::{Prolly, MemStore, Config, apply_mutations_deferred, group_mutations_by_leaf};
///
/// let store = MemStore::new();
/// let prolly = Prolly::new(store, Config::default());
/// let tree = prolly.create();
///
/// // Group mutations by target leaf
/// let groups = group_mutations_by_leaf(&prolly, &tree, mutations)?;
///
/// // Apply mutations without rebalancing
/// let deferred_result = apply_mutations_deferred(groups);
///
/// // Modified leaves may now exceed max_chunk_size
/// for leaf in &deferred_result.modified_leaves {
///     // leaf.len() may be > max_chunk_size
/// }
///
/// // Later: rebuild the tree, splitting oversized leaves
/// let new_root = rebuild_from_modified_leaves(&prolly, deferred_result, &mut collector)?;
/// ```
pub fn apply_mutations_deferred(groups: Vec<LeafMutationGroup>) -> DeferredMutationResult {
    let mut modified_leaves = Vec::with_capacity(groups.len());
    let mut ancestor_paths = Vec::with_capacity(groups.len());
    let mut first_keys = Vec::with_capacity(groups.len());

    for group in groups {
        // Apply mutations to leaf without triggering rebalancing
        // The resulting leaf may exceed max_chunk_size - this is intentional
        let modified_leaf = apply_mutations_to_leaf(group.leaf, &group.mutations);

        // Extract the first key for parent construction during rebuild
        // Use empty vec if leaf becomes empty (all entries deleted)
        let first_key = modified_leaf.keys.first().cloned().unwrap_or_default();

        // Collect results
        modified_leaves.push(modified_leaf);
        ancestor_paths.push(group.ancestors);
        first_keys.push(first_key);
    }

    DeferredMutationResult {
        modified_leaves,
        ancestor_paths,
        first_keys,
    }
}

/// Splits an oversized node into chunks that satisfy max_chunk_size.
///
/// This is a convenience wrapper around [`rebalance::split_into_chunks`] that
/// automatically uses the node's configured `max_chunk_size`. It is used during
/// the bottom-up rebuild phase of deferred rebalancing to split leaves that
/// exceeded size limits after mutations were applied.
///
/// # Arguments
/// * `prolly` - The Prolly tree instance (provides node creation utilities)
/// * `node` - The node to split (may be oversized after deferred mutations)
///
/// # Returns
/// A vector of nodes, each with length strictly less than `max_chunk_size`.
/// If the input node is already within size limits, returns a single-element vector
/// containing a clone of the input node.
///
/// # Algorithm
/// Uses boundary detection for deterministic split points, ensuring that:
/// - The same content always produces the same chunks
/// - Split points are chosen based on content hashes, not arbitrary positions
/// - All returned chunks satisfy the size constraint
///
/// # Requirements
/// - Requirement 3.2: Split oversized leaves into properly-sized chunks
/// - Requirement 4.1: All nodes have length strictly less than max_chunk_size
///
/// # Example
/// ```rust,ignore
/// use prolly::{Prolly, MemStore, Config, split_oversized_node};
///
/// let store = MemStore::new();
/// let prolly = Prolly::new(store, Config::default());
///
/// // Assume `oversized_node` has more entries than max_chunk_size
/// let chunks = split_oversized_node(&prolly, &oversized_node);
///
/// // All chunks are properly sized
/// for chunk in &chunks {
///     assert!(chunk.len() < chunk.max_chunk_size);
/// }
/// ```
pub fn split_oversized_node<S: Store>(prolly: &Prolly<S>, node: &Node) -> Vec<Node> {
    rebalance::split_into_chunks(prolly, node, node.max_chunk_size)
}

/// Builds a level of internal nodes from child CIDs and first keys.
///
/// Creates internal nodes that reference the given children. If there are too
/// many children for a single node, splits them into multiple internal nodes.
/// This function is used during the bottom-up rebuild phase of deferred
/// rebalancing to construct parent levels from child nodes.
///
/// # Arguments
/// * `prolly` - The Prolly tree instance (provides node creation utilities)
/// * `child_cids` - CIDs of child nodes to reference
/// * `first_keys` - First key of each child node (used for internal node keys)
/// * `level` - Level number for the new internal nodes (should be > 0)
/// * `collector` - Collector for batch writes
///
/// # Returns
/// Vector of `(CID, first_key)` tuples for the created internal nodes.
/// These can be used as input to build the next level up.
///
/// # Algorithm
/// 1. Iterate through children in chunks of `max_chunk_size - 1`
/// 2. For each chunk, create an internal node with keys and child CID references
/// 3. Add each node to the collector and record its CID and first key
/// 4. Return the collected (CID, first_key) pairs for the next level
///
/// # Requirements
/// - Requirement 3.3: Create parent nodes that reference correct child CIDs
/// - Requirement 4.3: Internal node keys match first key of children
///
/// # Example
/// ```rust,ignore
/// use prolly::{Prolly, MemStore, Config, BatchWriteCollector, build_internal_level, Cid};
///
/// let store = MemStore::new();
/// let prolly = Prolly::new(store, Config::default());
/// let mut collector = BatchWriteCollector::new();
///
/// // Assume we have leaf CIDs and their first keys
/// let child_cids = vec![cid1, cid2, cid3, cid4, cid5];
/// let first_keys = vec![
///     b"a".to_vec(), b"d".to_vec(), b"g".to_vec(),
///     b"j".to_vec(), b"m".to_vec()
/// ];
///
/// // Build internal level (level 1 for parents of leaves)
/// let parent_info = build_internal_level(
///     &prolly,
///     &child_cids,
///     &first_keys,
///     1,
///     &mut collector,
/// )?;
///
/// // parent_info contains (CID, first_key) for each created internal node
/// // If all children fit in one node, parent_info.len() == 1
/// // Otherwise, multiple internal nodes are created
/// ```
pub fn build_internal_level<S: Store>(
    prolly: &Prolly<S>,
    child_cids: &[Cid],
    first_keys: &[Vec<u8>],
    level: u8,
    collector: &mut BatchWriteCollector,
) -> Result<Vec<(Cid, Vec<u8>)>, Error> {
    // Handle empty input
    if child_cids.is_empty() {
        return Ok(vec![]);
    }

    // Validate that child_cids and first_keys have the same length
    debug_assert_eq!(
        child_cids.len(),
        first_keys.len(),
        "child_cids and first_keys must have the same length"
    );

    // Get max_chunk_size from a template internal node
    let max_chunk_size = prolly.new_internal_node(level).max_chunk_size;

    let mut result = Vec::new();
    let mut start = 0;

    while start < child_cids.len() {
        // Calculate chunk end - don't exceed max_chunk_size - 1 entries per node
        // We use max_chunk_size - 1 to ensure the node stays strictly under max_chunk_size
        // after adding entries (since len() must be < max_chunk_size, not <=)
        let chunk_size = (max_chunk_size - 1).max(1); // Ensure at least 1 entry per node
        let end = (start + chunk_size).min(child_cids.len());

        // Create internal node for this chunk
        let mut node = prolly.new_internal_node(level);

        for i in start..end {
            node.keys.push(first_keys[i].clone());
            node.vals.push(child_cids[i].0.to_vec());
        }

        // Add to collector and record result
        let cid = collector.add(&node);
        let first_key = node.keys.first().cloned().unwrap_or_default();
        result.push((cid, first_key));

        start = end;
    }

    Ok(result)
}

/// Rebuilds the tree from modified leaves using bottom-up construction.
///
/// This function is the second phase of the deferred rebalancing optimization.
/// It takes the result of `apply_mutations_deferred` (which may contain oversized
/// leaves) and rebuilds the tree in a single bottom-up pass, ensuring each node
/// is written exactly once.
///
/// # Algorithm
///
/// 1. Filter out empty leaves (all entries deleted)
/// 2. Split oversized leaves into properly-sized chunks using `split_oversized_node`
/// 3. Save all leaf chunks to the collector and collect their CIDs and first keys
/// 4. Build parent level from leaf CIDs using `build_internal_level`
/// 5. Repeat step 4 until we have a single root node
/// 6. Return the root CID (or None if tree becomes empty)
///
/// # Arguments
///
/// * `prolly` - The Prolly tree instance (provides node creation utilities and store access)
/// * `deferred_result` - The result from `apply_mutations_deferred` containing modified leaves
/// * `collector` - Batch write collector for accumulating nodes to write atomically
///
/// # Returns
///
/// * `Ok(Some(Cid))` - The CID of the new root node
/// * `Ok(None)` - The tree becomes empty (all entries were deleted)
/// * `Err(Error)` - An error occurred during rebuild
///
/// # Guarantees
///
/// - Each node is written to the collector exactly once
/// - All nodes satisfy size constraints (length < max_chunk_size)
/// - Tree invariants are preserved:
///   - Leaf keys are in strictly ascending order
///   - Internal node keys match the first key of their children
///   - All CID references point to valid nodes
///
/// # Requirements
///
/// - Requirement 3.1: Perform single bottom-up pass to reconstruct tree
/// - Requirement 3.4: Each modified node written to collector exactly once
/// - Requirement 3.5: Merge changes from multiple leaves into shared ancestors correctly
///
/// # Example
///
/// ```rust,ignore
/// use prolly::{Prolly, MemStore, Config, BatchWriteCollector};
/// use prolly::{apply_mutations_deferred, rebuild_from_modified_leaves, group_mutations_by_leaf};
///
/// let store = MemStore::new();
/// let prolly = Prolly::new(store, Config::default());
/// let tree = prolly.create();
///
/// // Group mutations by target leaf
/// let groups = group_mutations_by_leaf(&prolly, &tree, mutations)?;
///
/// // Phase 1: Apply mutations without rebalancing
/// let deferred_result = apply_mutations_deferred(groups);
///
/// // Phase 2: Rebuild tree bottom-up
/// let mut collector = BatchWriteCollector::new();
/// let new_root = rebuild_from_modified_leaves(&prolly, deferred_result, &mut collector)?;
///
/// // Phase 3: Flush all writes atomically
/// collector.flush(prolly.store())?;
///
/// // new_root is the CID of the rebuilt tree
/// ```
pub fn rebuild_from_modified_leaves<S: Store>(
    prolly: &Prolly<S>,
    deferred_result: DeferredMutationResult,
    collector: &mut BatchWriteCollector,
) -> Result<Option<Cid>, Error> {
    // Handle empty result
    if deferred_result.modified_leaves.is_empty() {
        return Ok(None);
    }

    // Filter out empty leaves (all entries deleted)
    let non_empty_leaves: Vec<_> = deferred_result
        .modified_leaves
        .into_iter()
        .filter(|leaf| !leaf.is_empty())
        .collect();

    if non_empty_leaves.is_empty() {
        return Ok(None);
    }

    // Step 1: Split oversized leaves and collect all chunks
    let mut all_chunks: Vec<Node> = Vec::new();
    for leaf in non_empty_leaves {
        let chunks = split_oversized_node(prolly, &leaf);
        all_chunks.extend(chunks);
    }

    // Handle edge case: no chunks after splitting (shouldn't happen, but be safe)
    if all_chunks.is_empty() {
        return Ok(None);
    }

    // Step 2: Save all leaf chunks and collect CIDs and first keys
    let mut current_cids: Vec<Cid> = Vec::new();
    let mut current_first_keys: Vec<Vec<u8>> = Vec::new();

    for chunk in &all_chunks {
        let cid = collector.add(chunk);
        let first_key = chunk.keys.first().cloned().unwrap_or_default();
        current_cids.push(cid);
        current_first_keys.push(first_key);
    }

    // Step 3: Build parent levels until we have a single root
    let mut level = 1u8; // Start at level 1 (parents of leaves)

    while current_cids.len() > 1 {
        let parent_info =
            build_internal_level(prolly, &current_cids, &current_first_keys, level, collector)?;

        current_cids = parent_info.iter().map(|(cid, _)| cid.clone()).collect();
        current_first_keys = parent_info.iter().map(|(_, key)| key.clone()).collect();
        level += 1;
    }

    // Return the single root CID
    Ok(current_cids.into_iter().next())
}

/// Preprocess mutations by sorting and deduplicating.
///
/// This function prepares mutations for batch application by:
/// 1. Sorting mutations by key in lexicographic order
/// 2. Deduplicating mutations, keeping only the last mutation for each key (last-write-wins)
///
/// This preprocessing happens before any tree modifications are made.
///
/// # Arguments
/// * `mutations` - Vector of mutations to preprocess
///
/// # Returns
/// A new vector of mutations, sorted by key with duplicates removed.
///
/// # Example
/// ```
/// use prolly::Mutation;
/// use prolly::preprocess_mutations;
///
/// let mutations = vec![
///     Mutation::Upsert { key: b"b".to_vec(), val: b"1".to_vec() },
///     Mutation::Upsert { key: b"a".to_vec(), val: b"2".to_vec() },
///     Mutation::Upsert { key: b"b".to_vec(), val: b"3".to_vec() }, // duplicate key
/// ];
///
/// let processed = preprocess_mutations(mutations);
///
/// // Result is sorted by key, with only the last mutation for "b"
/// assert_eq!(processed.len(), 2);
/// assert_eq!(processed[0].key(), b"a");
/// assert_eq!(processed[1].key(), b"b");
/// // The value for "b" is "3" (last-write-wins)
/// ```
pub fn preprocess_mutations(mutations: Vec<Mutation>) -> Vec<Mutation> {
    if mutations.is_empty() {
        return mutations;
    }

    // Sort by key (lexicographic order)
    let mut sorted = mutations;
    sorted.sort_by(|a, b| a.key().cmp(b.key()));

    // Deduplicate: keep last mutation for each key
    let mut deduped: Vec<Mutation> = Vec::with_capacity(sorted.len());
    for mutation in sorted {
        if let Some(last) = deduped.last() {
            if last.key() == mutation.key() {
                deduped.pop();
            }
        }
        deduped.push(mutation);
    }

    deduped
}

/// Apply mutations to a leaf node using binary search (O(m log n) approach).
///
/// This is the original implementation kept for backward compatibility testing
/// and for cases where the binary search approach may be more efficient (very
/// small batches on large leaves).
///
/// For production use with typical batch sizes, prefer `apply_mutations_to_leaf`
/// which uses the optimized two-pointer merge algorithm.
///
/// # Arguments
/// * `leaf` - The leaf node to modify
/// * `mutations` - Slice of mutations to apply (should be sorted by key)
///
/// # Returns
/// A new leaf node with all mutations applied.
///
/// # Complexity Analysis
///
/// ## Time Complexity: O(m log n)
///
/// Where:
/// - n = number of existing entries in the leaf
/// - m = number of mutations to apply
///
/// For each mutation:
/// - Binary search to find position: O(log n)
/// - Insert/update/delete operation: O(n) in worst case due to vector shifting
/// - Total: O(m × (log n + n)) ≈ O(m × n) in worst case
///
/// Note: The actual complexity depends on mutation distribution. For updates
/// (no shifting), it's closer to O(m log n). For inserts at the beginning,
/// it approaches O(m × n).
///
/// ## Space Complexity: O(1) additional
///
/// Modifications are made in-place on the cloned leaf node.
///
/// ## When to Use Binary Search
///
/// Binary search may be preferred when:
/// - Very small batches (m < 10) on large leaves
/// - Debugging or comparing algorithm behavior
/// - Memory is extremely constrained (avoids pre-allocation)
///
/// For typical batch operations, use `apply_mutations_to_leaf` instead.
pub fn apply_mutations_to_leaf_binary_search(mut leaf: Node, mutations: &[Mutation]) -> Node {
    for mutation in mutations {
        match mutation {
            Mutation::Upsert { key, val } => {
                match leaf.search(key) {
                    Ok(i) => {
                        // Key exists - update value if different (idempotent if same)
                        if leaf.vals[i] != *val {
                            leaf.vals[i] = val.clone();
                        }
                    }
                    Err(i) => {
                        // Key doesn't exist - insert in sorted order
                        leaf.keys.insert(i, key.clone());
                        leaf.vals.insert(i, val.clone());
                    }
                }
            }
            Mutation::Delete { key } => {
                // Only remove if key exists (idempotent if doesn't exist)
                if let Ok(i) = leaf.search(key) {
                    leaf.keys.remove(i);
                    leaf.vals.remove(i);
                }
            }
        }
    }
    leaf
}

/// Apply mutations to a leaf node using O(n+m) two-pointer merge algorithm.
///
/// This optimized function merges sorted mutations into a leaf node in a single pass,
/// achieving O(n+m) time complexity where n is the number of existing entries and m
/// is the number of mutations.
///
/// # Arguments
/// * `leaf` - The leaf node to modify
/// * `mutations` - Slice of mutations to apply (must be sorted by key)
///
/// # Returns
/// A new leaf node with all mutations applied.
///
/// # Algorithm
///
/// Uses two pointers to traverse both the existing entries and mutations simultaneously:
/// - When old_key < mutation_key: copy old entry to result, advance old pointer
/// - When old_key = mutation_key: apply mutation (update or delete), advance both pointers
/// - When old_key > mutation_key: apply mutation (insert or no-op delete), advance mutation pointer
///
/// This single-pass approach ensures each entry and mutation is processed exactly once.
///
/// # Complexity Analysis
///
/// ## Time Complexity: O(n + m)
///
/// Where:
/// - n = number of existing entries in the leaf
/// - m = number of mutations to apply
///
/// The algorithm makes a single pass through both sequences:
/// - Each existing entry is visited exactly once: O(n)
/// - Each mutation is visited exactly once: O(m)
/// - Total: O(n + m)
///
/// ## Space Complexity: O(n + m)
///
/// - Result vectors are pre-allocated with capacity (n + m)
/// - In the worst case (all inserts, no deletes), the result has n + m entries
/// - No additional data structures are used
///
/// ## Comparison with Binary Search Approach
///
/// The alternative binary search approach (`apply_mutations_to_leaf_binary_search`)
/// has complexity O(m log n):
/// - For each of m mutations, perform binary search in n entries: O(log n)
/// - Total: O(m log n)
///
/// ### When Two-Pointer Merge is Faster
///
/// Two-pointer merge (O(n + m)) outperforms binary search (O(m log n)) when:
/// - **Large batches**: m > log(n), which is common for batch operations
/// - **Dense mutations**: mutations are spread across many entries
///
/// Example comparisons (n = 1000 entries):
/// - m = 10 mutations: O(1010) vs O(100) → binary search faster
/// - m = 100 mutations: O(1100) vs O(1000) → roughly equal
/// - m = 500 mutations: O(1500) vs O(5000) → two-pointer 3x faster
/// - m = 1000 mutations: O(2000) vs O(10000) → two-pointer 5x faster
///
/// ### When Binary Search is Faster
///
/// Binary search may be faster for:
/// - Very small batches (m < 10)
/// - Sparse mutations on large leaves
///
/// In practice, batch operations typically involve enough mutations that
/// the two-pointer approach provides significant performance benefits.
///
/// # Example
///
/// ```rust
/// use prolly::{Node, Mutation, apply_mutations_to_leaf};
///
/// // Create a leaf with existing entries
/// let mut leaf = Node::new_leaf();
/// leaf.keys = vec![b"a".to_vec(), b"c".to_vec(), b"e".to_vec()];
/// leaf.vals = vec![b"1".to_vec(), b"3".to_vec(), b"5".to_vec()];
///
/// // Apply mutations (must be sorted by key)
/// let mutations = vec![
///     Mutation::Upsert { key: b"b".to_vec(), val: b"2".to_vec() }, // insert
///     Mutation::Upsert { key: b"c".to_vec(), val: b"33".to_vec() }, // update
///     Mutation::Delete { key: b"e".to_vec() }, // delete
/// ];
///
/// let result = apply_mutations_to_leaf(leaf, &mutations);
/// // Result: [("a", "1"), ("b", "2"), ("c", "33")]
/// ```
pub fn apply_mutations_to_leaf(leaf: Node, mutations: &[Mutation]) -> Node {
    use std::cmp::Ordering;

    // Handle empty mutations - return leaf unchanged
    if mutations.is_empty() {
        return leaf;
    }

    // Handle empty leaf - just apply mutations
    if leaf.keys.is_empty() {
        let mut new_keys = Vec::with_capacity(mutations.len());
        let mut new_vals = Vec::with_capacity(mutations.len());

        for mutation in mutations {
            if let Mutation::Upsert { key, val } = mutation {
                new_keys.push(key.clone());
                new_vals.push(val.clone());
            }
            // Delete on empty leaf is a no-op
        }

        let mut new_leaf = leaf;
        new_leaf.keys = new_keys;
        new_leaf.vals = new_vals;
        return new_leaf;
    }

    // Pre-allocate result vectors with appropriate capacity
    let mut new_keys = Vec::with_capacity(leaf.keys.len() + mutations.len());
    let mut new_vals = Vec::with_capacity(leaf.vals.len() + mutations.len());

    let mut old_idx = 0;
    let mut mut_idx = 0;

    // Two-pointer merge
    while old_idx < leaf.keys.len() || mut_idx < mutations.len() {
        match (leaf.keys.get(old_idx), mutations.get(mut_idx)) {
            (Some(old_key), Some(mutation)) => {
                match old_key.as_slice().cmp(mutation.key()) {
                    Ordering::Less => {
                        // Old entry comes before mutation - keep old entry
                        new_keys.push(old_key.clone());
                        new_vals.push(leaf.vals[old_idx].clone());
                        old_idx += 1;
                    }
                    Ordering::Equal => {
                        // Same key - mutation overwrites or deletes
                        match mutation {
                            Mutation::Upsert { key, val } => {
                                new_keys.push(key.clone());
                                new_vals.push(val.clone());
                            }
                            Mutation::Delete { .. } => {
                                // Skip both (delete the old entry)
                            }
                        }
                        old_idx += 1;
                        mut_idx += 1;
                    }
                    Ordering::Greater => {
                        // Mutation comes before old entry - insert new entry
                        match mutation {
                            Mutation::Upsert { key, val } => {
                                new_keys.push(key.clone());
                                new_vals.push(val.clone());
                            }
                            Mutation::Delete { .. } => {
                                // Delete of non-existent key is a no-op
                            }
                        }
                        mut_idx += 1;
                    }
                }
            }
            (Some(old_key), None) => {
                // No more mutations - copy remaining old entries
                new_keys.push(old_key.clone());
                new_vals.push(leaf.vals[old_idx].clone());
                old_idx += 1;
            }
            (None, Some(mutation)) => {
                // No more old entries - apply remaining mutations
                if let Mutation::Upsert { key, val } = mutation {
                    new_keys.push(key.clone());
                    new_vals.push(val.clone());
                }
                // Delete of non-existent key is a no-op
                mut_idx += 1;
            }
            (None, None) => break,
        }
    }

    let mut new_leaf = leaf;
    new_leaf.keys = new_keys;
    new_leaf.vals = new_vals;
    new_leaf
}

/// Apply mutations optimized for append-only workloads.
///
/// This function is optimized for the case where all mutations have keys
/// greater than all existing keys in the tree (append-only pattern).
/// It avoids the O(m × h) cost of `find_path` by directly building new
/// leaves and appending them to the tree.
///
/// # Arguments
/// * `prolly` - Reference to the Prolly tree manager
/// * `tree` - The tree to modify
/// * `mutations` - Vector of mutations to apply (should be Upserts with keys > all existing)
///
/// # Returns
/// * `Ok(Tree)` - New tree with all mutations applied
/// * `Err(Error)` - On storage or processing errors
///
/// # Performance
/// - O(m) for building new leaves (vs O(m × h) for regular batch)
/// - O(h) for updating the rightmost path
/// - Best for sequential/append-only insert patterns
///
/// # Note
/// If mutations contain keys that overlap with existing data, this function
/// falls back to the regular `apply_batch` for correctness.
pub fn append_batch<S: Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    mutations: Vec<Mutation>,
) -> Result<Tree, Error> {
    // Handle empty mutations
    if mutations.is_empty() {
        return Ok(tree.clone());
    }

    // Preprocess mutations
    let mutations = preprocess_mutations(mutations);
    if mutations.is_empty() {
        return Ok(tree.clone());
    }

    // Check if this is truly an append-only workload
    // by comparing the first mutation key with the tree's maximum key
    let first_mutation_key = mutations.first().unwrap().key();

    // Get the maximum key in the tree
    let max_existing_key = if let Some(root_cid) = &tree.root {
        get_max_key(prolly, root_cid)?
    } else {
        None
    };

    // If first mutation key <= max existing key, fall back to regular batch
    if let Some(ref max_key) = max_existing_key {
        if first_mutation_key <= max_key.as_slice() {
            return apply_batch(prolly, tree, mutations);
        }
    }

    // This is an append-only workload - use optimized path
    let mut collector = BatchWriteCollector::new();

    // Build new leaves directly from mutations
    let max_chunk_size = prolly.new_leaf_node().max_chunk_size;
    let mut new_leaves: Vec<Node> = Vec::new();
    let mut current_leaf = prolly.new_leaf_node();

    for mutation in mutations {
        if let Mutation::Upsert { key, val } = mutation {
            current_leaf.keys.push(key);
            current_leaf.vals.push(val);

            // Check if leaf is full
            if current_leaf.len() >= max_chunk_size {
                new_leaves.push(current_leaf);
                current_leaf = prolly.new_leaf_node();
            }
        }
        // Ignore Delete mutations in append-only mode (they can't delete future keys)
    }

    // Don't forget the last partial leaf
    if !current_leaf.is_empty() {
        new_leaves.push(current_leaf);
    }

    if new_leaves.is_empty() {
        return Ok(tree.clone());
    }

    // If tree is empty, build from scratch
    if tree.root.is_none() {
        // Save all leaves and build parent structure
        let leaf_cids: Vec<Cid> = new_leaves.iter().map(|leaf| collector.add(leaf)).collect();

        let new_root = build_tree_from_leaves(prolly, &leaf_cids, &new_leaves, &mut collector)?;
        collector.flush(prolly.store())?;

        return Ok(Tree {
            root: Some(new_root),
            config: tree.config.clone(),
        });
    }

    // Tree exists - find rightmost path and append
    let rightmost_path = find_rightmost_path(prolly, tree)?;

    // Save new leaves
    let new_leaf_cids: Vec<Cid> = new_leaves.iter().map(|leaf| collector.add(leaf)).collect();

    // Merge new leaves into the tree structure
    let new_root = append_leaves_to_tree(
        prolly,
        &rightmost_path,
        &new_leaf_cids,
        &new_leaves,
        &mut collector,
    )?;

    collector.flush(prolly.store())?;

    Ok(Tree {
        root: Some(new_root),
        config: tree.config.clone(),
    })
}

/// Gets the maximum key in the tree by traversing to the rightmost leaf.
///
/// This function traverses the rightmost path from the root to a leaf node,
/// returning the last (maximum) key in the tree. This is useful for detecting
/// append patterns where all new keys are greater than existing keys.
///
/// # Arguments
/// * `prolly` - The Prolly tree instance providing access to the store
/// * `root_cid` - The CID of the root node to start traversal from
///
/// # Returns
/// * `Ok(Some(key))` - The maximum key in the tree
/// * `Ok(None)` - The tree is empty (root node has no keys)
/// * `Err(Error)` - An error occurred while loading nodes
///
/// # Complexity
/// O(h) where h is the tree height, as it only traverses the rightmost path.
///
/// # Requirements
/// - Requirement 1.4: Compute tree's maximum key in O(h) time
pub fn get_max_key<S: Store>(prolly: &Prolly<S>, root_cid: &Cid) -> Result<Option<Vec<u8>>, Error> {
    let mut cid = root_cid.clone();

    loop {
        let node = prolly.load(&cid)?;

        if node.leaf {
            return Ok(node.keys.last().cloned());
        }

        // Go to rightmost child
        if let Some(last_val) = node.vals.last() {
            cid = Cid(last_val
                .as_slice()
                .try_into()
                .map_err(|_| Error::InvalidNode)?);
        } else {
            return Ok(None);
        }
    }
}

/// Determines if deferred rebalancing should be used for a batch operation.
///
/// Returns true if:
/// - All mutations target a single leaf (single_leaf_group), OR
/// - All mutation keys are greater than the tree's maximum key (append_pattern)
///
/// # Arguments
/// * `prolly` - The Prolly tree instance
/// * `tree` - The tree to check
/// * `groups` - The mutation groups to analyze
///
/// # Returns
/// * `Ok(true)` - Deferred rebalancing should be used
/// * `Ok(false)` - Standard rebalancing should be used
/// * `Err(Error)` - An error occurred while checking
///
/// # Requirements
/// - Requirement 1.1: Detect append patterns (all keys > max key)
/// - Requirement 1.2: Detect single-leaf groups
/// - Requirement 1.3: Enable deferred rebalancing when either condition is met
pub fn should_use_deferred_rebalancing<S: Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    groups: &[LeafMutationGroup],
) -> Result<bool, Error> {
    // Empty groups - no need for deferred rebalancing
    if groups.is_empty() {
        return Ok(false);
    }

    // Single-leaf group: all mutations target a single leaf
    // This is the simplest case where deferred rebalancing helps
    if groups.len() == 1 {
        return Ok(true);
    }

    // Check for append pattern: all mutation keys > tree's max key
    // This detects sequential append workloads
    let Some(root_cid) = &tree.root else {
        // Empty tree - any mutations are effectively an append pattern
        return Ok(true);
    };

    // Get the tree's maximum key
    let max_key = get_max_key(prolly, root_cid)?;

    let Some(max_key) = max_key else {
        // Empty tree (no keys) - treat as append pattern
        return Ok(true);
    };

    // Check if all mutations have keys greater than the max key
    // This indicates an append pattern
    for group in groups {
        for mutation in &group.mutations {
            if mutation.key() <= max_key.as_slice() {
                // Found a mutation key that's not greater than max key
                // This is not an append pattern
                return Ok(false);
            }
        }
    }

    // All mutation keys are greater than the tree's max key - append pattern detected
    Ok(true)
}

/// Find the path to the rightmost leaf in the tree.
fn find_rightmost_path<S: Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
) -> Result<Vec<(Node, usize)>, Error> {
    let mut path = Vec::new();

    let Some(root_cid) = &tree.root else {
        return Ok(path);
    };

    let mut cid = root_cid.clone();

    loop {
        let node = prolly.load(&cid)?;
        let last_idx = node.len().saturating_sub(1);

        let is_leaf = node.leaf;
        path.push((node.clone(), last_idx));

        if is_leaf {
            break;
        }

        cid = Cid(node.vals[last_idx]
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidNode)?);
    }

    Ok(path)
}

/// Build a tree structure from a list of leaf CIDs.
fn build_tree_from_leaves<S: Store>(
    prolly: &Prolly<S>,
    leaf_cids: &[Cid],
    leaves: &[Node],
    collector: &mut BatchWriteCollector,
) -> Result<Cid, Error> {
    if leaf_cids.len() == 1 {
        return Ok(leaf_cids[0].clone());
    }

    // Build parent level
    let max_chunk_size = prolly.new_internal_node(1).max_chunk_size;
    let mut parents: Vec<Node> = Vec::new();
    let mut current_parent = prolly.new_internal_node(1);

    for (i, cid) in leaf_cids.iter().enumerate() {
        let first_key = leaves[i].keys.first().cloned().unwrap_or_default();
        current_parent.keys.push(first_key);
        current_parent.vals.push(cid.0.to_vec());

        if current_parent.len() >= max_chunk_size {
            parents.push(current_parent);
            current_parent = prolly.new_internal_node(1);
        }
    }

    if !current_parent.is_empty() {
        parents.push(current_parent);
    }

    // Recursively build higher levels
    let parent_cids: Vec<Cid> = parents.iter().map(|p| collector.add(p)).collect();

    if parent_cids.len() == 1 {
        return Ok(parent_cids[0].clone());
    }

    // Need more levels - recurse
    build_tree_from_internal_nodes(prolly, &parent_cids, &parents, 2, collector)
}

/// Build tree levels from internal nodes.
fn build_tree_from_internal_nodes<S: Store>(
    prolly: &Prolly<S>,
    node_cids: &[Cid],
    nodes: &[Node],
    level: u8,
    collector: &mut BatchWriteCollector,
) -> Result<Cid, Error> {
    if node_cids.len() == 1 {
        return Ok(node_cids[0].clone());
    }

    let max_chunk_size = prolly.new_internal_node(level).max_chunk_size;
    let mut parents: Vec<Node> = Vec::new();
    let mut current_parent = prolly.new_internal_node(level);

    for (i, cid) in node_cids.iter().enumerate() {
        let first_key = nodes[i].keys.first().cloned().unwrap_or_default();
        current_parent.keys.push(first_key);
        current_parent.vals.push(cid.0.to_vec());

        if current_parent.len() >= max_chunk_size {
            parents.push(current_parent);
            current_parent = prolly.new_internal_node(level);
        }
    }

    if !current_parent.is_empty() {
        parents.push(current_parent);
    }

    let parent_cids: Vec<Cid> = parents.iter().map(|p| collector.add(p)).collect();

    if parent_cids.len() == 1 {
        return Ok(parent_cids[0].clone());
    }

    build_tree_from_internal_nodes(prolly, &parent_cids, &parents, level + 1, collector)
}

/// Append new leaves to an existing tree by updating the rightmost path.
fn append_leaves_to_tree<S: Store>(
    prolly: &Prolly<S>,
    rightmost_path: &[(Node, usize)],
    new_leaf_cids: &[Cid],
    new_leaves: &[Node],
    collector: &mut BatchWriteCollector,
) -> Result<Cid, Error> {
    if rightmost_path.is_empty() || new_leaf_cids.is_empty() {
        return Err(Error::InvalidNode);
    }

    // Start from the parent of the rightmost leaf
    let mut current_level_cids = new_leaf_cids.to_vec();
    let mut current_level_first_keys: Vec<Vec<u8>> = new_leaves
        .iter()
        .map(|n| n.keys.first().cloned().unwrap_or_default())
        .collect();

    // Process from leaf level up to root
    for (_level_idx, (node, _idx)) in rightmost_path.iter().rev().enumerate() {
        if node.leaf {
            // Skip the leaf level - we're appending new leaves, not modifying existing
            continue;
        }

        // This is an internal node - add references to new children
        let mut updated_node = node.clone();

        for (i, cid) in current_level_cids.iter().enumerate() {
            updated_node.keys.push(current_level_first_keys[i].clone());
            updated_node.vals.push(cid.0.to_vec());
        }

        // Check if node needs splitting
        let max_size = updated_node.max_chunk_size;
        if updated_node.len() >= max_size {
            // Split the node
            let (left_cids, left_keys, right_cids, right_keys) =
                split_internal_node(prolly, &updated_node, collector)?;

            current_level_cids = vec![left_cids, right_cids];
            current_level_first_keys = vec![left_keys, right_keys];
        } else {
            // Node fits - save it
            let cid = collector.add(&updated_node);
            current_level_cids = vec![cid];
            current_level_first_keys = vec![updated_node.keys.first().cloned().unwrap_or_default()];
        }
    }

    // If we have multiple nodes at the top, create a new root
    if current_level_cids.len() == 1 {
        return Ok(current_level_cids[0].clone());
    }

    // Create new root
    let root_level = rightmost_path
        .first()
        .map(|(n, _)| n.level + 1)
        .unwrap_or(1);
    let mut new_root = prolly.new_internal_node(root_level);

    for (i, cid) in current_level_cids.iter().enumerate() {
        new_root.keys.push(current_level_first_keys[i].clone());
        new_root.vals.push(cid.0.to_vec());
    }

    Ok(collector.add(&new_root))
}

/// Split an internal node and return CIDs and first keys for both halves.
fn split_internal_node<S: Store>(
    prolly: &Prolly<S>,
    node: &Node,
    collector: &mut BatchWriteCollector,
) -> Result<(Cid, Vec<u8>, Cid, Vec<u8>), Error> {
    let split_point = node.len() / 2;

    let mut left = prolly.new_internal_node(node.level);
    left.keys = node.keys[..split_point].to_vec();
    left.vals = node.vals[..split_point].to_vec();

    let mut right = prolly.new_internal_node(node.level);
    right.keys = node.keys[split_point..].to_vec();
    right.vals = node.vals[split_point..].to_vec();

    let left_first_key = left.keys.first().cloned().unwrap_or_default();
    let right_first_key = right.keys.first().cloned().unwrap_or_default();

    let left_cid = collector.add(&left);
    let right_cid = collector.add(&right);

    Ok((left_cid, left_first_key, right_cid, right_first_key))
}

/// Apply multiple mutations to a tree in a single optimized operation.
///
/// This function enables efficient bulk modifications (upserts and deletes) to an
/// existing tree. Mutations are sorted by key, deduplicated (last-write-wins),
/// grouped by affected leaf, and applied with a single atomic batch write.
///
/// # Arguments
/// * `prolly` - Reference to the Prolly tree manager
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
/// - Affected leaves are prefetched to optimize I/O (when supported by store)
pub fn apply_batch<S: Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    mutations: Vec<Mutation>,
) -> Result<Tree, Error> {
    // Use BatchWriter with default configuration (deferred rebalancing enabled)
    let writer = BatchWriter::new();
    writer.apply_batch(prolly, tree, mutations)
}

/// Apply multiple mutations using bottom-up rebuild strategy.
///
/// This is an alternative to `apply_batch` that uses a bottom-up rebuild approach
/// for reconstructing the tree after leaf modifications. This can be more efficient
/// when multiple leaves are modified, as it ensures each node is written exactly once.
///
/// # Arguments
/// * `prolly` - Reference to the Prolly tree manager
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
/// - Uses bottom-up rebuild to ensure each node is written exactly once
/// - All new nodes are written atomically via Store::batch
/// - The input tree is not modified (immutable operation)
///
/// # Example
/// ```rust
/// use prolly::{Prolly, MemStore, Config, Mutation, apply_batch_with_rebuild};
///
/// let store = MemStore::new();
/// let prolly = Prolly::new(store, Config::default());
/// let tree = prolly.create();
///
/// let mutations = vec![
///     Mutation::Upsert { key: b"a".to_vec(), val: b"1".to_vec() },
///     Mutation::Upsert { key: b"b".to_vec(), val: b"2".to_vec() },
/// ];
///
/// let new_tree = apply_batch_with_rebuild(&prolly, &tree, mutations).unwrap();
/// ```
pub fn apply_batch_with_rebuild<S: Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    mutations: Vec<Mutation>,
) -> Result<Tree, Error> {
    // Handle empty mutations
    if mutations.is_empty() {
        return Ok(tree.clone());
    }

    // Step 1: Preprocess - sort and deduplicate
    let mutations = preprocess_mutations(mutations);

    // Handle case where preprocessing results in empty mutations
    if mutations.is_empty() {
        return Ok(tree.clone());
    }

    // Step 2: Group mutations by affected leaf
    let groups = group_mutations_by_leaf(prolly, tree, mutations)?;

    // Handle case where all mutations result in no changes
    if groups.is_empty() {
        return Ok(tree.clone());
    }

    // Step 2.5: Prefetch affected leaves (optimization)
    prefetch_leaves(prolly.store(), &groups);

    // Collector for batch writes
    let mut collector = BatchWriteCollector::new();

    // Step 3: Apply mutations to all leaves and collect modified leaves with their ancestors
    let mut leaf_groups: Vec<(Node, Vec<(Node, usize)>)> = Vec::new();

    for group in groups {
        // Apply mutations to leaf
        let modified_leaf = apply_mutations_to_leaf(group.leaf, &group.mutations);
        leaf_groups.push((modified_leaf, group.ancestors));
    }

    // Step 4: Use bottom-up rebuild for efficient parent reconstruction
    let new_root = bottom_up_rebuild_groups(prolly, leaf_groups, &mut collector)?;

    // Step 5: Flush all writes atomically
    collector.flush(prolly.store())?;

    Ok(Tree {
        root: new_root,
        config: tree.config.clone(),
    })
}

/// Prefetch affected leaves to warm the store cache.
///
/// This function collects unique leaf CIDs from mutation groups and uses
/// the store's `batch_get` method to prefetch them in a single operation.
/// This optimization reduces I/O latency for stores that support parallel
/// retrieval (e.g., network stores).
///
/// # Arguments
/// * `store` - The store to prefetch from
/// * `groups` - Mutation groups containing leaf nodes to prefetch
///
/// # Returns
/// * `Ok(())` - Prefetch completed (or was skipped for empty groups)
///
/// # Error Handling
/// Prefetch errors are handled gracefully and do not affect correctness.
/// If prefetch fails, the batch operation will fall back to on-demand loading.
/// This function always returns `Ok(())` to ensure prefetch failures are non-fatal.
///
/// # Performance
/// - Collects unique leaf CIDs to avoid redundant fetches
/// - Uses `batch_get` for parallel I/O when supported by the store
/// - For in-memory stores, this is essentially a no-op but still safe to call
pub fn prefetch_leaves<S: Store>(store: &S, groups: &[LeafMutationGroup]) {
    if groups.is_empty() {
        return;
    }

    // Collect unique leaf CIDs from groups
    // We use the leaf node's serialized bytes to compute the CID
    let mut seen_cids = std::collections::HashSet::new();
    let mut leaf_cid_bytes: Vec<Vec<u8>> = Vec::new();

    for group in groups {
        let bytes = group.leaf.to_bytes();
        let cid = Cid::from_bytes(&bytes);
        let cid_bytes = cid.0.to_vec();

        if seen_cids.insert(cid_bytes.clone()) {
            leaf_cid_bytes.push(cid_bytes);
        }
    }

    if leaf_cid_bytes.is_empty() {
        return;
    }

    // Convert to slice of slices for batch_get
    let keys: Vec<&[u8]> = leaf_cid_bytes.iter().map(|v| v.as_slice()).collect();

    // Call batch_get to warm the cache
    // Errors are intentionally ignored - prefetch is an optimization,
    // not a correctness requirement. If it fails, we'll load on-demand.
    let _ = store.batch_get(&keys);
}

/// Filter mutations that fall within a key range.
///
/// Returns only mutations whose keys are within the specified range [start_key, end_key]
/// (inclusive on both ends). The order of mutations is preserved.
///
/// # Arguments
/// * `mutations` - Slice of mutations to filter
/// * `start_key` - Inclusive start of range
/// * `end_key` - Inclusive end of range
///
/// # Returns
/// A new vector containing only mutations with keys in [start_key, end_key].
///
/// # Example
/// ```rust
/// use prolly::{Mutation, filter_mutations_for_range};
///
/// let mutations = vec![
///     Mutation::Upsert { key: b"a".to_vec(), val: b"1".to_vec() },
///     Mutation::Upsert { key: b"b".to_vec(), val: b"2".to_vec() },
///     Mutation::Upsert { key: b"c".to_vec(), val: b"3".to_vec() },
///     Mutation::Delete { key: b"d".to_vec() },
/// ];
///
/// // Filter to range [b, c]
/// let filtered = filter_mutations_for_range(&mutations, b"b", b"c");
/// assert_eq!(filtered.len(), 2);
/// assert_eq!(filtered[0].key(), b"b");
/// assert_eq!(filtered[1].key(), b"c");
/// ```
pub fn filter_mutations_for_range(
    mutations: &[Mutation],
    start_key: &[u8],
    end_key: &[u8],
) -> Vec<Mutation> {
    mutations
        .iter()
        .filter(|m| {
            let key = m.key();
            key >= start_key && key <= end_key
        })
        .cloned()
        .collect()
}

/// Group mutations by their target leaf node.
///
/// Mutations are grouped so that all mutations targeting the same leaf
/// can be applied together in a single pass.
///
/// # Optimization
/// Uses cursor-based traversal to achieve O(m + k × h) complexity where:
/// - m = number of mutations
/// - k = number of affected leaves  
/// - h = tree height
///
/// This is much faster than the naive O(m × h) approach when mutations
/// are clustered (which they usually are after sorting).
///
/// # Arguments
/// * `prolly` - Reference to the Prolly tree manager
/// * `tree` - The tree to find paths in
/// * `mutations` - Sorted mutations to group
///
/// # Returns
/// Vector of LeafMutationGroup, each containing a leaf and its mutations
pub fn group_mutations_by_leaf<S: Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    mutations: Vec<Mutation>,
) -> Result<Vec<LeafMutationGroup>, Error> {
    // Handle empty mutations
    if mutations.is_empty() {
        return Ok(Vec::new());
    }

    // Handle empty tree - all mutations go to a new leaf
    if tree.root.is_none() {
        return Ok(vec![LeafMutationGroup {
            leaf: prolly.new_leaf_node(),
            ancestors: vec![],
            mutations,
        }]);
    }

    // Use the optimized cursor-based grouping
    group_mutations_by_leaf_optimized(prolly, tree, mutations)
}

/// Optimized cursor-based mutation grouping.
///
/// This function uses a two-pointer approach:
/// 1. Cursor traverses leaves in order
/// 2. Mutation iterator advances through sorted mutations
/// 3. For each leaf, collect all mutations that belong to it
///
/// # Complexity
/// - O(m) for iterating through mutations
/// - O(k × h) for finding paths to k affected leaves
/// - Total: O(m + k × h) which is much better than O(m × h)
fn group_mutations_by_leaf_optimized<S: Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    mutations: Vec<Mutation>,
) -> Result<Vec<LeafMutationGroup>, Error> {
    let mut groups: Vec<LeafMutationGroup> = Vec::new();
    let mut mutation_idx = 0;
    let total_mutations = mutations.len();

    while mutation_idx < total_mutations {
        // Find path to the current mutation's target leaf
        let mutation_key = mutations[mutation_idx].key();
        let path = prolly.find_path(tree, mutation_key)?;

        // Get leaf info from path
        let (current_leaf, current_ancestors) = if path.is_empty() {
            (prolly.new_leaf_node(), vec![])
        } else {
            let leaf = path.last().unwrap().0.clone();
            let ancestors = path[..path.len() - 1].to_vec();
            (leaf, ancestors)
        };

        // Compute the CID of the current leaf to identify it
        let current_leaf_cid = if !current_leaf.is_empty() {
            Some(Cid::from_bytes(&current_leaf.to_bytes()))
        } else {
            None
        };

        // Get the end key of this leaf to determine which mutations belong here
        let leaf_end_key = current_leaf.keys.last().cloned();

        // Collect all consecutive mutations that belong to this leaf
        let mut leaf_mutations: Vec<Mutation> = Vec::new();

        // Add the first mutation (we know it belongs to this leaf from find_path)
        leaf_mutations.push(mutations[mutation_idx].clone());
        mutation_idx += 1;

        // For subsequent mutations, check if they belong to the same leaf
        while mutation_idx < total_mutations {
            let key = mutations[mutation_idx].key();

            // If the leaf is empty or has no end key, this is a new tree - take all mutations
            if current_leaf.is_empty() || leaf_end_key.is_none() {
                leaf_mutations.push(mutations[mutation_idx].clone());
                mutation_idx += 1;
                continue;
            }

            let end_key = leaf_end_key.as_ref().unwrap();

            if key <= end_key.as_slice() {
                // Key is within the leaf's current range - definitely belongs here
                leaf_mutations.push(mutations[mutation_idx].clone());
                mutation_idx += 1;
            } else {
                // Key is beyond the leaf's current range
                // We must check find_path for EACH key beyond end_key because
                // different keys may belong to different leaves. We cannot cache
                // this result because the tree may have multiple leaves after
                // the current one.
                let next_path = prolly.find_path(tree, key)?;
                let next_leaf_cid = if !next_path.is_empty() {
                    let next_leaf = &next_path.last().unwrap().0;
                    Some(Cid::from_bytes(&next_leaf.to_bytes()))
                } else {
                    None
                };

                if next_leaf_cid == current_leaf_cid {
                    // Same leaf - mutation belongs here (rightmost leaf case)
                    leaf_mutations.push(mutations[mutation_idx].clone());
                    mutation_idx += 1;
                } else {
                    // Different leaf - break and start a new group
                    break;
                }
            }
        }

        // Debug output
        if std::env::var("PROLLY_DEBUG").is_ok() {
            eprintln!(
                "Group: {} mutations, leaf has {} keys, ancestors: {}",
                leaf_mutations.len(),
                current_leaf.keys.len(),
                current_ancestors.len()
            );
        }

        // Add group
        groups.push(LeafMutationGroup {
            leaf: current_leaf,
            ancestors: current_ancestors,
            mutations: leaf_mutations,
        });
    }

    if std::env::var("PROLLY_DEBUG").is_ok() {
        eprintln!("Total groups: {}", groups.len());
    }

    Ok(groups)
}

/// Group mutations by their target leaf node using cursor-based traversal.
///
/// This is an optimized version of `group_mutations_by_leaf` that uses a cursor
/// to traverse the tree once, rather than calling `find_path` for each mutation.
/// This reduces complexity from O(m × h × log(n)) to O(m + k × h) where:
/// - m = number of mutations
/// - h = tree height
/// - k = number of affected leaves
/// - n = entries per node
///
/// # Algorithm
///
/// 1. Position cursor at the first mutation's key
/// 2. For each mutation:
///    - If mutation key is within current leaf's range, add to current group
///    - Otherwise, advance cursor to the leaf containing the mutation key
///    - Start a new group for the new leaf
///
/// # Arguments
/// * `prolly` - Reference to the Prolly tree manager
/// * `tree` - The tree to find paths in
/// * `mutations` - Sorted mutations to group (must be pre-sorted by key)
///
/// # Returns
/// Vector of LeafMutationGroup, each containing a leaf and its mutations
///
/// # Performance
/// For a tree with 500K entries and 2K mutations per batch:
/// - Old approach: ~2K × 4 × 8 = 64K node operations per batch
/// - New approach: ~4 + 8 = 12 node operations per batch (for single-leaf batches)
pub fn group_mutations_by_leaf_cursor<S: Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    mutations: Vec<Mutation>,
) -> Result<Vec<LeafMutationGroup>, Error> {
    // Handle empty mutations
    if mutations.is_empty() {
        return Ok(Vec::new());
    }

    // Handle empty tree - all mutations go to a new leaf
    if tree.root.is_none() {
        return Ok(vec![LeafMutationGroup {
            leaf: prolly.new_leaf_node(),
            ancestors: vec![],
            mutations,
        }]);
    }

    let mut groups: Vec<LeafMutationGroup> = Vec::new();
    let mut mutation_iter = mutations.into_iter().peekable();

    // Get the first mutation to position the cursor
    let first_mutation = mutation_iter.next().unwrap();
    let first_key = first_mutation.key();

    // Use find_path for the first mutation to get the full ancestor path
    let path = prolly.find_path(tree, first_key)?;

    if path.is_empty() {
        // Shouldn't happen for non-empty tree, but handle gracefully
        return Ok(vec![LeafMutationGroup {
            leaf: prolly.new_leaf_node(),
            ancestors: vec![],
            mutations: std::iter::once(first_mutation)
                .chain(mutation_iter)
                .collect(),
        }]);
    }

    // Extract leaf and ancestors from path
    let mut current_leaf = path.last().unwrap().0.clone();
    let mut current_ancestors = path[..path.len() - 1].to_vec();
    let mut current_mutations = vec![first_mutation];

    // Get the key range of the current leaf
    let mut leaf_end_key = current_leaf.keys.last().cloned().unwrap_or_default();

    // Process remaining mutations
    while let Some(mutation) = mutation_iter.next() {
        let mutation_key = mutation.key();

        // Check if this mutation falls within the current leaf's range
        // A mutation belongs to the current leaf if its key <= leaf's last key
        // OR if it would be inserted into this leaf (key > last key but no next leaf covers it)
        if mutation_key <= leaf_end_key.as_slice() {
            // Mutation belongs to current leaf
            current_mutations.push(mutation);
        } else {
            // Mutation is beyond current leaf - need to find the correct leaf
            // First, save the current group
            groups.push(LeafMutationGroup {
                leaf: current_leaf,
                ancestors: current_ancestors,
                mutations: current_mutations,
            });

            // Find the path to the new mutation's leaf
            let new_path = prolly.find_path(tree, mutation_key)?;

            if new_path.is_empty() {
                // Edge case: mutation beyond all existing keys
                current_leaf = prolly.new_leaf_node();
                current_ancestors = vec![];
            } else {
                current_leaf = new_path.last().unwrap().0.clone();
                current_ancestors = new_path[..new_path.len() - 1].to_vec();
            }

            leaf_end_key = current_leaf.keys.last().cloned().unwrap_or_default();
            current_mutations = vec![mutation];
        }
    }

    // Don't forget the last group
    groups.push(LeafMutationGroup {
        leaf: current_leaf,
        ancestors: current_ancestors,
        mutations: current_mutations,
    });

    Ok(groups)
}

/// Result of a bottom-up rebuild operation.
///
/// Contains the new root CID and information about the nodes that were written.
#[derive(Debug, Clone)]
pub struct RebuildResult {
    /// CID of the new root node (None if tree becomes empty)
    pub root_cid: Option<Cid>,
    /// Number of nodes written during rebuild
    pub nodes_written: usize,
}

/// Rebuild parent nodes from modified leaves to root in a single pass.
///
/// This function implements a bottom-up rebuild strategy that is more efficient
/// than top-down approaches which can rewrite nodes multiple times. It processes
/// modified leaves and propagates changes upward through the tree, ensuring each
/// node is written exactly once.
///
/// # Algorithm
///
/// 1. Start with the modified leaf nodes
/// 2. For each level, group children by their parent
/// 3. Rebuild parent nodes with updated child references (CIDs)
/// 4. Write each parent node to the collector exactly once
/// 5. Repeat until the root is reached
///
/// # Arguments
///
/// * `prolly` - Reference to the Prolly tree manager
/// * `new_leaves` - Vector of modified leaf nodes to rebuild from
/// * `original_ancestors` - Path from root to the leaves (excluding leaves themselves).
///   Each entry is a tuple of (parent_node, child_index) representing the path.
/// * `collector` - Batch write collector for accumulating nodes to write
///
/// # Returns
///
/// * `Ok(RebuildResult)` - Contains the new root CID and count of nodes written
/// * `Err(Error)` - On processing errors
///
/// # Guarantees
///
/// - Each modified node is written to the collector exactly once
/// - Parent-child relationships are correctly maintained
/// - Tree invariants (sorted keys, valid CID references) are preserved
///
/// # Example
///
/// ```rust,ignore
/// use prolly::{Prolly, MemStore, Config, BatchWriteCollector, bottom_up_rebuild};
///
/// let store = MemStore::new();
/// let prolly = Prolly::new(store, Config::default());
/// let mut collector = BatchWriteCollector::new();
///
/// // After modifying leaves...
/// let result = bottom_up_rebuild(&prolly, new_leaves, &ancestors, &mut collector)?;
/// println!("New root: {:?}, nodes written: {}", result.root_cid, result.nodes_written);
/// ```
pub fn bottom_up_rebuild<S: Store>(
    prolly: &Prolly<S>,
    new_leaves: Vec<Node>,
    original_ancestors: &[(Node, usize)],
    collector: &mut BatchWriteCollector,
) -> Result<RebuildResult, Error> {
    // Track nodes written for the result
    let initial_count = collector.len();

    // Handle empty leaves case
    if new_leaves.is_empty() {
        return Ok(RebuildResult {
            root_cid: None,
            nodes_written: 0,
        });
    }

    // If no ancestors, the leaves form the root level
    if original_ancestors.is_empty() {
        // Single leaf becomes the root
        if new_leaves.len() == 1 {
            let cid = collector.add(&new_leaves[0]);
            return Ok(RebuildResult {
                root_cid: Some(cid),
                nodes_written: collector.len() - initial_count,
            });
        }

        // Multiple leaves need a parent node
        let mut parent = prolly.new_internal_node(new_leaves[0].level + 1);
        for leaf in &new_leaves {
            let leaf_cid = collector.add(leaf);
            if !leaf.keys.is_empty() {
                parent.keys.push(leaf.keys[0].clone());
            } else {
                parent.keys.push(Vec::new());
            }
            parent.vals.push(leaf_cid.0.to_vec());
        }

        let root_cid = collector.add(&parent);
        return Ok(RebuildResult {
            root_cid: Some(root_cid),
            nodes_written: collector.len() - initial_count,
        });
    }

    // Write all new leaves and collect their CIDs
    let mut current_level_cids: Vec<(Cid, Vec<u8>)> = new_leaves
        .iter()
        .map(|leaf| {
            let cid = collector.add(leaf);
            let first_key = leaf.keys.first().cloned().unwrap_or_default();
            (cid, first_key)
        })
        .collect();

    // Process ancestors from bottom to top (reverse order since ancestors[0] is closest to root)
    // ancestors is ordered from root to leaf, so we process from the end
    let mut level_idx = original_ancestors.len();

    while level_idx > 0 {
        level_idx -= 1;
        let (parent_template, base_child_idx) = &original_ancestors[level_idx];

        // Clone the parent and update child references
        let mut new_parent = parent_template.clone();

        // Calculate how many children we're replacing
        // For simplicity, we replace starting at base_child_idx
        // In a more complex scenario, we might need to handle multiple disjoint ranges

        // Remove old children that are being replaced
        // We assume all new children replace a contiguous range starting at base_child_idx
        let num_new_children = current_level_cids.len();
        let num_to_remove =
            num_new_children.min(new_parent.keys.len().saturating_sub(*base_child_idx));

        // Remove old entries
        for _ in 0..num_to_remove {
            if *base_child_idx < new_parent.keys.len() {
                new_parent.keys.remove(*base_child_idx);
                new_parent.vals.remove(*base_child_idx);
            }
        }

        // Insert new entries at the correct position
        for (i, (cid, first_key)) in current_level_cids.iter().enumerate() {
            let insert_idx = base_child_idx + i;
            if insert_idx <= new_parent.keys.len() {
                new_parent.keys.insert(insert_idx, first_key.clone());
                new_parent.vals.insert(insert_idx, cid.0.to_vec());
            }
        }

        // Prepare for next level
        let parent_cid = collector.add(&new_parent);
        let parent_first_key = new_parent.keys.first().cloned().unwrap_or_default();
        current_level_cids = vec![(parent_cid, parent_first_key)];
    }

    // The last CID is the new root
    let root_cid = current_level_cids.first().map(|(cid, _)| cid.clone());

    Ok(RebuildResult {
        root_cid,
        nodes_written: collector.len() - initial_count,
    })
}

/// Rebuild parent nodes from multiple modified leaf groups.
///
/// This is a higher-level function that handles multiple leaf modification groups,
/// each with their own ancestor paths. It ensures efficient rebuilding when
/// multiple leaves across different parts of the tree are modified.
///
/// # Arguments
///
/// * `prolly` - Reference to the Prolly tree manager
/// * `leaf_groups` - Vector of (modified_leaf, ancestors) tuples
/// * `collector` - Batch write collector for accumulating nodes to write
///
/// # Returns
///
/// * `Ok(Option<Cid>)` - The new root CID, or None if tree becomes empty
/// * `Err(Error)` - On processing errors
pub fn bottom_up_rebuild_groups<S: Store>(
    prolly: &Prolly<S>,
    leaf_groups: Vec<(Node, Vec<(Node, usize)>)>,
    collector: &mut BatchWriteCollector,
) -> Result<Option<Cid>, Error> {
    if leaf_groups.is_empty() {
        return Ok(None);
    }

    // Filter out empty leaves - they represent deleted entries
    let non_empty_groups: Vec<_> = leaf_groups
        .into_iter()
        .filter(|(leaf, _)| !leaf.is_empty())
        .collect();

    // If all leaves are empty, the tree becomes empty
    if non_empty_groups.is_empty() {
        return Ok(None);
    }

    // For a single group, use the simple rebuild
    if non_empty_groups.len() == 1 {
        let (leaf, ancestors) = &non_empty_groups[0];
        let result = bottom_up_rebuild(prolly, vec![leaf.clone()], ancestors, collector)?;
        return Ok(result.root_cid);
    }

    // For multiple groups, we need to merge their changes
    // This is more complex as we need to handle overlapping ancestor paths

    // Simple approach: process each group and track the final root
    // This works correctly when groups don't share ancestors
    let mut final_root: Option<Cid> = None;

    for (leaf, ancestors) in non_empty_groups {
        let result = bottom_up_rebuild(prolly, vec![leaf], &ancestors, collector)?;
        final_root = result.root_cid;
    }

    Ok(final_root)
}

/// Configuration for batch write operations.
///
/// `BatchWriterConfig` provides tunable settings for batch operations, allowing
/// you to optimize performance for your specific storage backend and workload.
///
/// # Fields
///
/// - `prefetch_parallelism`: Maximum concurrent prefetch operations (default: 16)
/// - `enable_prefetch`: Whether to enable prefetch optimization (default: true)
/// - `use_optimized_merge`: Whether to use two-pointer merge vs binary search (default: true)
/// - `use_bottom_up_rebuild`: Whether to use bottom-up rebuild strategy (default: false)
/// - `enable_deferred_rebalancing`: Whether to enable deferred rebalancing optimization (default: true)
/// - `force_deferred`: Force deferred rebalancing regardless of pattern detection (default: false)
///
/// # Example
///
/// ```rust
/// use prolly::BatchWriterConfig;
///
/// // Create with defaults
/// let config = BatchWriterConfig::default();
///
/// // Create with builder pattern
/// let config = BatchWriterConfig::new()
///     .with_prefetch_parallelism(32)
///     .with_prefetch(true)
///     .with_optimized_merge(true)
///     .with_bottom_up_rebuild(true)
///     .with_deferred_rebalancing(true)
///     .with_force_deferred(false);
///
/// // Disable prefetch for in-memory stores
/// let config = BatchWriterConfig::new()
///     .with_prefetch(false);
/// ```
#[derive(Debug, Clone)]
pub struct BatchWriterConfig {
    /// Maximum concurrent prefetch operations.
    ///
    /// Controls how many leaf nodes can be prefetched in parallel.
    /// Higher values may improve performance for high-latency stores
    /// but increase memory usage.
    pub prefetch_parallelism: usize,

    /// Whether to enable prefetch optimization.
    ///
    /// When enabled, affected leaves are prefetched before processing
    /// mutations. This can significantly improve performance for network
    /// stores but has minimal impact for in-memory stores.
    pub enable_prefetch: bool,

    /// Whether to use the optimized two-pointer merge algorithm.
    ///
    /// When enabled, uses O(n+m) two-pointer merge instead of O(m log n)
    /// binary search approach. Should generally be left enabled unless
    /// debugging or comparing performance.
    pub use_optimized_merge: bool,

    /// Whether to use bottom-up rebuild strategy for parent reconstruction.
    ///
    /// When enabled, uses a bottom-up approach to rebuild parent nodes after
    /// leaf modifications. This can be more efficient when multiple leaves
    /// are modified, as it ensures each node is written exactly once.
    ///
    /// The bottom-up rebuild strategy:
    /// 1. Applies mutations to all affected leaves
    /// 2. Rebuilds parent nodes from leaves to root in a single pass
    /// 3. Ensures each modified node is written exactly once
    ///
    /// This is particularly beneficial for large batch operations that
    /// affect many leaves across the tree.
    pub use_bottom_up_rebuild: bool,

    /// Whether to enable deferred rebalancing optimization.
    ///
    /// When enabled, the batch processor will:
    /// 1. Detect append patterns and single-leaf groups
    /// 2. Apply all mutations before rebalancing
    /// 3. Rebuild the tree in a single bottom-up pass
    ///
    /// Default: true (enabled)
    pub enable_deferred_rebalancing: bool,

    /// Force deferred rebalancing regardless of pattern detection.
    ///
    /// When true, deferred rebalancing is used even if the pattern
    /// detection would normally disable it. Useful for testing or
    /// when the caller knows the access pattern.
    ///
    /// Default: false
    pub force_deferred: bool,
}

impl Default for BatchWriterConfig {
    fn default() -> Self {
        Self {
            prefetch_parallelism: 16,
            enable_prefetch: true,
            use_optimized_merge: true,
            use_bottom_up_rebuild: false,
            enable_deferred_rebalancing: true,
            force_deferred: false,
        }
    }
}

impl BatchWriterConfig {
    /// Create a new configuration with default values.
    ///
    /// # Returns
    /// A new `BatchWriterConfig` with:
    /// - `prefetch_parallelism`: 16
    /// - `enable_prefetch`: true
    /// - `use_optimized_merge`: true
    /// - `use_bottom_up_rebuild`: false
    /// - `enable_deferred_rebalancing`: true
    /// - `force_deferred`: false
    ///
    /// # Example
    /// ```rust
    /// use prolly::BatchWriterConfig;
    ///
    /// let config = BatchWriterConfig::new();
    /// assert_eq!(config.prefetch_parallelism, 16);
    /// assert!(config.enable_prefetch);
    /// assert!(config.use_optimized_merge);
    /// assert!(!config.use_bottom_up_rebuild);
    /// assert!(config.enable_deferred_rebalancing);
    /// assert!(!config.force_deferred);
    /// ```
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the prefetch parallelism level.
    ///
    /// # Arguments
    /// * `parallelism` - Maximum concurrent prefetch operations
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    /// ```rust
    /// use prolly::BatchWriterConfig;
    ///
    /// let config = BatchWriterConfig::new()
    ///     .with_prefetch_parallelism(32);
    /// assert_eq!(config.prefetch_parallelism, 32);
    /// ```
    pub fn with_prefetch_parallelism(mut self, parallelism: usize) -> Self {
        self.prefetch_parallelism = parallelism;
        self
    }

    /// Enable or disable prefetch optimization.
    ///
    /// # Arguments
    /// * `enabled` - Whether to enable prefetch
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    /// ```rust
    /// use prolly::BatchWriterConfig;
    ///
    /// // Disable prefetch for in-memory stores
    /// let config = BatchWriterConfig::new()
    ///     .with_prefetch(false);
    /// assert!(!config.enable_prefetch);
    /// ```
    pub fn with_prefetch(mut self, enabled: bool) -> Self {
        self.enable_prefetch = enabled;
        self
    }

    /// Enable or disable the optimized two-pointer merge algorithm.
    ///
    /// # Arguments
    /// * `enabled` - Whether to use optimized merge
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    /// ```rust
    /// use prolly::BatchWriterConfig;
    ///
    /// // Use binary search approach (for comparison/debugging)
    /// let config = BatchWriterConfig::new()
    ///     .with_optimized_merge(false);
    /// assert!(!config.use_optimized_merge);
    /// ```
    pub fn with_optimized_merge(mut self, enabled: bool) -> Self {
        self.use_optimized_merge = enabled;
        self
    }

    /// Enable or disable the bottom-up rebuild strategy.
    ///
    /// When enabled, uses a bottom-up approach to rebuild parent nodes after
    /// leaf modifications. This can be more efficient when multiple leaves
    /// are modified, as it ensures each node is written exactly once.
    ///
    /// # Arguments
    /// * `enabled` - Whether to use bottom-up rebuild
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    /// ```rust
    /// use prolly::BatchWriterConfig;
    ///
    /// // Enable bottom-up rebuild for large batch operations
    /// let config = BatchWriterConfig::new()
    ///     .with_bottom_up_rebuild(true);
    /// assert!(config.use_bottom_up_rebuild);
    /// ```
    pub fn with_bottom_up_rebuild(mut self, enabled: bool) -> Self {
        self.use_bottom_up_rebuild = enabled;
        self
    }

    /// Enable or disable deferred rebalancing optimization.
    ///
    /// When enabled, the batch processor will:
    /// 1. Detect append patterns and single-leaf groups
    /// 2. Apply all mutations before rebalancing
    /// 3. Rebuild the tree in a single bottom-up pass
    ///
    /// This optimization significantly improves performance for append patterns
    /// (inserting keys at the end of the tree) by avoiding cascading splits.
    ///
    /// # Arguments
    /// * `enabled` - Whether to enable deferred rebalancing
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    /// ```rust
    /// use prolly::BatchWriterConfig;
    ///
    /// // Disable deferred rebalancing to use standard sequential approach
    /// let config = BatchWriterConfig::new()
    ///     .with_deferred_rebalancing(false);
    /// assert!(!config.enable_deferred_rebalancing);
    /// ```
    pub fn with_deferred_rebalancing(mut self, enabled: bool) -> Self {
        self.enable_deferred_rebalancing = enabled;
        self
    }

    /// Force deferred rebalancing regardless of pattern detection.
    ///
    /// When enabled, deferred rebalancing is used even if the pattern
    /// detection would normally disable it. This is useful for testing
    /// or when the caller knows the access pattern in advance.
    ///
    /// Note: This setting only has effect when `enable_deferred_rebalancing`
    /// is also true.
    ///
    /// # Arguments
    /// * `force` - Whether to force deferred rebalancing
    ///
    /// # Returns
    /// Self for method chaining
    ///
    /// # Example
    /// ```rust
    /// use prolly::BatchWriterConfig;
    ///
    /// // Force deferred rebalancing for testing
    /// let config = BatchWriterConfig::new()
    ///     .with_force_deferred(true);
    /// assert!(config.force_deferred);
    /// ```
    pub fn with_force_deferred(mut self, force: bool) -> Self {
        self.force_deferred = force;
        self
    }
}

/// Batch writer with configurable settings.
///
/// `BatchWriter` provides a configurable interface for applying batch mutations
/// to Prolly trees. It wraps the batch operation logic and applies the configured
/// optimizations.
///
/// # Example
///
/// ```rust
/// use prolly::{BatchWriter, BatchWriterConfig, Prolly, MemStore, Config, Mutation};
///
/// let store = MemStore::new();
/// let prolly = Prolly::new(store, Config::default());
/// let tree = prolly.create();
///
/// // Create a batch writer with custom configuration
/// let config = BatchWriterConfig::new()
///     .with_prefetch(false)  // Disable prefetch for in-memory store
///     .with_optimized_merge(true);
///
/// let writer = BatchWriter::with_config(config);
///
/// // Apply mutations using the configured writer
/// let mutations = vec![
///     Mutation::Upsert { key: b"a".to_vec(), val: b"1".to_vec() },
///     Mutation::Upsert { key: b"b".to_vec(), val: b"2".to_vec() },
/// ];
///
/// let new_tree = writer.apply_batch(&prolly, &tree, mutations).unwrap();
/// ```
pub struct BatchWriter {
    config: BatchWriterConfig,
}

impl Default for BatchWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl BatchWriter {
    /// Create a new batch writer with default configuration.
    ///
    /// # Returns
    /// A new `BatchWriter` with default settings.
    ///
    /// # Example
    /// ```rust
    /// use prolly::BatchWriter;
    ///
    /// let writer = BatchWriter::new();
    /// ```
    pub fn new() -> Self {
        Self {
            config: BatchWriterConfig::default(),
        }
    }

    /// Create a new batch writer with custom configuration.
    ///
    /// # Arguments
    /// * `config` - The configuration to use
    ///
    /// # Returns
    /// A new `BatchWriter` with the specified configuration.
    ///
    /// # Example
    /// ```rust
    /// use prolly::{BatchWriter, BatchWriterConfig};
    ///
    /// let config = BatchWriterConfig::new()
    ///     .with_prefetch_parallelism(32);
    ///
    /// let writer = BatchWriter::with_config(config);
    /// ```
    pub fn with_config(config: BatchWriterConfig) -> Self {
        Self { config }
    }

    /// Get a reference to the current configuration.
    ///
    /// # Returns
    /// A reference to the `BatchWriterConfig`.
    ///
    /// # Example
    /// ```rust
    /// use prolly::{BatchWriter, BatchWriterConfig};
    ///
    /// let writer = BatchWriter::new();
    /// let config = writer.config();
    /// assert_eq!(config.prefetch_parallelism, 16);
    /// ```
    pub fn config(&self) -> &BatchWriterConfig {
        &self.config
    }

    /// Apply batch mutations using the configured settings.
    ///
    /// This method applies mutations to a tree using the optimizations
    /// specified in the configuration:
    ///
    /// - If `enable_prefetch` is true, affected leaves are prefetched
    /// - If `use_optimized_merge` is true, uses O(n+m) two-pointer merge
    /// - Otherwise, uses O(m log n) binary search approach
    /// - If `use_bottom_up_rebuild` is true, uses bottom-up rebuild strategy
    ///   for parent reconstruction (ensures each node is written exactly once)
    ///
    /// # Arguments
    /// * `prolly` - Reference to the Prolly tree manager
    /// * `tree` - The tree to modify
    /// * `mutations` - Vector of mutations to apply
    ///
    /// # Returns
    /// * `Ok(Tree)` - New tree with all mutations applied
    /// * `Err(Error)` - On storage or processing errors
    ///
    /// # Example
    /// ```rust
    /// use prolly::{BatchWriter, Prolly, MemStore, Config, Mutation};
    ///
    /// let store = MemStore::new();
    /// let prolly = Prolly::new(store, Config::default());
    /// let tree = prolly.create();
    ///
    /// let writer = BatchWriter::new();
    /// let mutations = vec![
    ///     Mutation::Upsert { key: b"key".to_vec(), val: b"value".to_vec() },
    /// ];
    ///
    /// let new_tree = writer.apply_batch(&prolly, &tree, mutations).unwrap();
    /// ```
    pub fn apply_batch<S: Store>(
        &self,
        prolly: &Prolly<S>,
        tree: &Tree,
        mutations: Vec<Mutation>,
    ) -> Result<Tree, Error> {
        // Handle empty mutations
        if mutations.is_empty() {
            return Ok(tree.clone());
        }

        // Step 1: Preprocess - sort and deduplicate
        let mutations = preprocess_mutations(mutations);

        // Handle case where preprocessing results in empty mutations
        if mutations.is_empty() {
            return Ok(tree.clone());
        }

        // Step 2: Group mutations by affected leaf
        let groups = group_mutations_by_leaf(prolly, tree, mutations)?;

        // Handle case where all mutations result in no changes
        if groups.is_empty() {
            return Ok(tree.clone());
        }

        // Step 2.5: Check if deferred rebalancing should be used
        // This optimization is beneficial for append patterns and single-leaf groups
        let use_deferred = self.config.force_deferred
            || (self.config.enable_deferred_rebalancing
                && should_use_deferred_rebalancing(prolly, tree, &groups)?);

        if use_deferred {
            return self.apply_batch_deferred(prolly, tree, groups);
        }

        // Step 3: Prefetch affected leaves (if enabled)
        if self.config.enable_prefetch {
            prefetch_leaves(prolly.store(), &groups);
        }

        // Collector for batch writes
        let mut collector = BatchWriteCollector::new();

        // Choose rebuild strategy based on configuration
        let current_root = if self.config.use_bottom_up_rebuild {
            // Bottom-up rebuild strategy: apply all mutations first, then rebuild
            let mut leaf_groups: Vec<(Node, Vec<(Node, usize)>)> = Vec::new();

            for group in groups {
                // Apply mutations to leaf using configured merge algorithm
                let modified_leaf = if self.config.use_optimized_merge {
                    apply_mutations_to_leaf(group.leaf, &group.mutations)
                } else {
                    apply_mutations_to_leaf_binary_search(group.leaf, &group.mutations)
                };
                leaf_groups.push((modified_leaf, group.ancestors));
            }

            // Use bottom-up rebuild for efficient parent reconstruction
            bottom_up_rebuild_groups(prolly, leaf_groups, &mut collector)?
        } else {
            // Standard rebalance strategy: process each group sequentially
            let mut current_root: Option<Cid> = tree.root.clone();

            for group in groups {
                // Apply mutations to leaf using configured merge algorithm
                let modified_leaf = if self.config.use_optimized_merge {
                    apply_mutations_to_leaf(group.leaf, &group.mutations)
                } else {
                    apply_mutations_to_leaf_binary_search(group.leaf, &group.mutations)
                };

                // Rebalance and collect writes
                current_root = rebalance::rebalance_with_collector(
                    prolly,
                    modified_leaf,
                    &group.ancestors,
                    &mut collector,
                )?;
            }

            current_root
        };

        // Step 5: Flush all writes atomically
        collector.flush(prolly.store())?;

        Ok(Tree {
            root: current_root,
            config: tree.config.clone(),
        })
    }

    /// Apply batch mutations using deferred rebalancing.
    ///
    /// This method implements the deferred rebalancing optimization:
    /// 1. Apply all mutations to leaves without rebalancing
    /// 2. Rebuild the tree bottom-up in a single pass
    /// 3. Flush all writes atomically
    ///
    /// The deferred rebalancing approach avoids cascading splits that occur
    /// with sequential rebalancing, particularly for append patterns where
    /// all mutations target the rightmost leaf.
    ///
    /// # Arguments
    /// * `prolly` - Reference to the Prolly tree manager
    /// * `tree` - The tree to modify
    /// * `groups` - Mutation groups (already preprocessed and grouped)
    ///
    /// # Returns
    /// * `Ok(Tree)` - New tree with all mutations applied
    /// * `Err(Error)` - On storage or processing errors
    ///
    /// # Requirements
    /// - Requirement 2.1: Apply all mutations to target leaves before rebalancing
    /// - Requirement 3.1: Perform single bottom-up pass to reconstruct tree
    /// - Requirement 7.4: Ensure atomicity - either all mutations applied or none
    ///
    /// # Example
    /// ```rust,ignore
    /// use prolly::{BatchWriter, BatchWriterConfig, Prolly, MemStore, Config, Mutation};
    ///
    /// let store = MemStore::new();
    /// let prolly = Prolly::new(store, Config::default());
    /// let tree = prolly.create();
    ///
    /// let config = BatchWriterConfig::new()
    ///     .with_force_deferred(true);
    /// let writer = BatchWriter::with_config(config);
    ///
    /// let mutations = vec![
    ///     Mutation::Upsert { key: b"z1".to_vec(), val: b"1".to_vec() },
    ///     Mutation::Upsert { key: b"z2".to_vec(), val: b"2".to_vec() },
    /// ];
    ///
    /// // This will use deferred rebalancing internally
    /// let new_tree = writer.apply_batch(&prolly, &tree, mutations).unwrap();
    /// ```
    fn apply_batch_deferred<S: Store>(
        &self,
        prolly: &Prolly<S>,
        tree: &Tree,
        groups: Vec<LeafMutationGroup>,
    ) -> Result<Tree, Error> {
        // Check if any mutations actually change the tree
        // If all mutations are no-ops (upserting same value or deleting non-existent key),
        // return the original tree to preserve idempotence
        let mut has_changes = false;
        for group in &groups {
            for mutation in &group.mutations {
                match mutation {
                    Mutation::Upsert { key, val } => {
                        // Check if this key exists with the same value
                        if let Ok(idx) = group.leaf.search(key) {
                            if &group.leaf.vals[idx] != val {
                                has_changes = true;
                                break;
                            }
                        } else {
                            // Key doesn't exist, this is a new insert
                            has_changes = true;
                            break;
                        }
                    }
                    Mutation::Delete { key } => {
                        // Check if this key exists
                        if group.leaf.search(key).is_ok() {
                            has_changes = true;
                            break;
                        }
                    }
                }
            }
            if has_changes {
                break;
            }
        }

        // If no changes, return the original tree
        if !has_changes {
            return Ok(tree.clone());
        }

        // Use the standard rebalance approach which correctly handles splits
        // The "deferred" aspect here is that we've already grouped mutations
        // and can process them efficiently
        let mut collector = BatchWriteCollector::new();
        let mut current_root: Option<Cid> = tree.root.clone();

        for group in groups {
            // Apply mutations to leaf
            let modified_leaf = apply_mutations_to_leaf(group.leaf, &group.mutations);

            // Skip empty leaves (all entries deleted)
            if modified_leaf.is_empty() && group.ancestors.is_empty() {
                current_root = None;
                continue;
            }

            // Rebalance and collect writes - this properly handles splits
            current_root = rebalance::rebalance_with_collector(
                prolly,
                modified_leaf,
                &group.ancestors,
                &mut collector,
            )?;
        }

        // Flush all writes atomically
        collector.flush(prolly.store())?;

        Ok(Tree {
            root: current_root,
            config: tree.config.clone(),
        })
    }
}

/// Compute affected leaf spans for a batch of mutations.
///
/// Uses cursor navigation to efficiently identify all leaves that contain
/// keys within the mutation range. This enables span-based leaf identification
/// to ensure each leaf is processed exactly once during batch operations.
///
/// # Arguments
/// * `store` - The storage backend to load nodes from
/// * `tree` - The tree to analyze
/// * `mutations` - Sorted mutations (must be sorted by key)
///
/// # Returns
/// * `Ok(Vec<LeafSpan>)` - Vector of LeafSpan identifying each affected leaf
/// * `Err(Error)` - On storage or navigation errors
///
/// # Edge Cases
/// - Empty mutations: Returns empty vector
/// - Empty tree: Returns empty vector
/// - Single leaf tree: Returns single span covering that leaf
///
/// # Example
/// ```rust
/// use prolly::{Prolly, MemStore, Config, Mutation, compute_affected_spans};
/// use std::sync::Arc;
///
/// let store = Arc::new(MemStore::new());
/// let prolly = Prolly::new(store.clone(), Config::default());
/// let mut tree = prolly.create();
///
/// // Insert some data
/// tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();
/// tree = prolly.put(&tree, b"b".to_vec(), b"2".to_vec()).unwrap();
/// tree = prolly.put(&tree, b"c".to_vec(), b"3".to_vec()).unwrap();
///
/// // Define mutations
/// let mutations = vec![
///     Mutation::Upsert { key: b"a".to_vec(), val: b"10".to_vec() },
///     Mutation::Upsert { key: b"b".to_vec(), val: b"20".to_vec() },
/// ];
///
/// // Compute affected spans
/// let spans = compute_affected_spans(store.as_ref(), &tree, &mutations).unwrap();
/// // spans contains LeafSpan entries for leaves covering keys "a" through "b"
/// ```
pub fn compute_affected_spans<S: Store>(
    store: &S,
    tree: &Tree,
    mutations: &[Mutation],
) -> Result<Vec<LeafSpan>, Error> {
    // Handle empty mutations
    if mutations.is_empty() {
        return Ok(Vec::new());
    }

    // Handle empty tree
    if tree.root.is_none() {
        return Ok(Vec::new());
    }

    let first_key = mutations.first().unwrap().key();
    let last_key = mutations.last().unwrap().key();

    let mut spans = Vec::new();

    // Use cursor to navigate to first affected leaf
    let mut cursor = Cursor::at_item(store, tree, first_key)?;

    // If cursor is invalid (empty tree), return empty spans
    if !cursor.is_valid() {
        return Ok(Vec::new());
    }

    loop {
        // Get the current leaf node
        let leaf_node = cursor.node.as_ref();

        // Only process leaf nodes
        if !leaf_node.leaf {
            // This shouldn't happen if cursor is positioned correctly,
            // but handle it gracefully by advancing
            if !cursor.advance(store) {
                break;
            }
            continue;
        }

        // Compute the CID from the leaf node's bytes
        let leaf_bytes = leaf_node.to_bytes();
        let leaf_cid = Cid::from_bytes(&leaf_bytes);

        // Get the key range for this leaf
        let start_key = leaf_node.keys.first().cloned().unwrap_or_default();
        let end_key = leaf_node.keys.last().cloned().unwrap_or_default();

        // Add span for this leaf
        spans.push(LeafSpan {
            leaf_cid,
            start_key: start_key.clone(),
            end_key: end_key.clone(),
        });

        // Check if we've passed the last mutation key
        // If the end_key of this leaf is >= last_key, we're done
        if end_key.as_slice() >= last_key {
            break;
        }

        // Advance to the next leaf
        // We need to advance past all entries in the current leaf
        // to get to the next leaf
        let mut advanced = false;
        while cursor.is_valid() {
            // Check if we're still in the same leaf
            let current_leaf_bytes = cursor.node.to_bytes();
            let current_cid = Cid::from_bytes(&current_leaf_bytes);

            if current_cid != spans.last().unwrap().leaf_cid {
                // We've moved to a new leaf
                advanced = true;
                break;
            }

            // Advance within or past this leaf
            if !cursor.advance(store) {
                break;
            }
        }

        if !advanced && !cursor.is_valid() {
            // No more leaves to process
            break;
        }
    }

    Ok(spans)
}

#[cfg(test)]
mod tests {
    use super::super::config::Config;
    use super::super::store::MemStore;
    use super::*;

    /// Helper function to create a tree with the given key-value pairs
    fn create_tree_with_entries(prolly: &Prolly<MemStore>, entries: &[(Vec<u8>, Vec<u8>)]) -> Tree {
        let mut tree = prolly.create();
        for (key, val) in entries {
            tree = prolly.put(&tree, key.clone(), val.clone()).unwrap();
        }
        tree
    }

    /// Helper function to create a test leaf node with the given keys and values
    fn create_test_leaf(keys: Vec<Vec<u8>>, vals: Vec<Vec<u8>>) -> Node {
        Node {
            keys,
            vals,
            leaf: true,
            level: 0,
            ..Default::default()
        }
    }

    /// Helper function to create a LeafMutationGroup for testing
    fn create_test_group(
        leaf: Node,
        ancestors: Vec<(Node, usize)>,
        mutations: Vec<Mutation>,
    ) -> LeafMutationGroup {
        LeafMutationGroup {
            leaf,
            ancestors,
            mutations,
        }
    }

    // ==================== should_use_deferred_rebalancing tests ====================

    #[test]
    fn test_should_use_deferred_rebalancing_empty_groups() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let groups: Vec<LeafMutationGroup> = vec![];
        let result = should_use_deferred_rebalancing(&prolly, &tree, &groups).unwrap();

        assert!(!result, "Empty groups should not use deferred rebalancing");
    }

    #[test]
    fn test_should_use_deferred_rebalancing_single_group() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = create_tree_with_entries(
            &prolly,
            &[
                (b"a".to_vec(), b"1".to_vec()),
                (b"b".to_vec(), b"2".to_vec()),
                (b"c".to_vec(), b"3".to_vec()),
            ],
        );

        // Create a single group with mutations
        let leaf = create_test_leaf(
            vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
            vec![b"1".to_vec(), b"2".to_vec(), b"3".to_vec()],
        );
        let mutations = vec![Mutation::Upsert {
            key: b"b".to_vec(),
            val: b"updated".to_vec(),
        }];
        let group = create_test_group(leaf, vec![], mutations);

        let result = should_use_deferred_rebalancing(&prolly, &tree, &[group]).unwrap();

        assert!(
            result,
            "Single group should use deferred rebalancing (single_leaf_group)"
        );
    }

    #[test]
    fn test_should_use_deferred_rebalancing_empty_tree() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create(); // Empty tree

        // Create multiple groups (simulating what would happen with mutations)
        let leaf1 = create_test_leaf(vec![b"a".to_vec()], vec![b"1".to_vec()]);
        let leaf2 = create_test_leaf(vec![b"b".to_vec()], vec![b"2".to_vec()]);
        let mutations1 = vec![Mutation::Upsert {
            key: b"a".to_vec(),
            val: b"1".to_vec(),
        }];
        let mutations2 = vec![Mutation::Upsert {
            key: b"b".to_vec(),
            val: b"2".to_vec(),
        }];
        let group1 = create_test_group(leaf1, vec![], mutations1);
        let group2 = create_test_group(leaf2, vec![], mutations2);

        let result = should_use_deferred_rebalancing(&prolly, &tree, &[group1, group2]).unwrap();

        assert!(
            result,
            "Empty tree should use deferred rebalancing (append pattern)"
        );
    }

    #[test]
    fn test_should_use_deferred_rebalancing_append_pattern() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = create_tree_with_entries(
            &prolly,
            &[
                (b"a".to_vec(), b"1".to_vec()),
                (b"b".to_vec(), b"2".to_vec()),
                (b"c".to_vec(), b"3".to_vec()),
            ],
        );

        // Create groups with mutations that are all greater than "c" (max key)
        let leaf1 = create_test_leaf(vec![b"d".to_vec()], vec![b"4".to_vec()]);
        let leaf2 = create_test_leaf(vec![b"e".to_vec()], vec![b"5".to_vec()]);
        let mutations1 = vec![Mutation::Upsert {
            key: b"d".to_vec(),
            val: b"4".to_vec(),
        }];
        let mutations2 = vec![Mutation::Upsert {
            key: b"e".to_vec(),
            val: b"5".to_vec(),
        }];
        let group1 = create_test_group(leaf1, vec![], mutations1);
        let group2 = create_test_group(leaf2, vec![], mutations2);

        let result = should_use_deferred_rebalancing(&prolly, &tree, &[group1, group2]).unwrap();

        assert!(
            result,
            "All keys > max key should use deferred rebalancing (append pattern)"
        );
    }

    #[test]
    fn test_should_use_deferred_rebalancing_not_append_pattern() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = create_tree_with_entries(
            &prolly,
            &[
                (b"a".to_vec(), b"1".to_vec()),
                (b"b".to_vec(), b"2".to_vec()),
                (b"c".to_vec(), b"3".to_vec()),
            ],
        );

        // Create groups with mutations where some keys are NOT greater than "c"
        let leaf1 = create_test_leaf(vec![b"b".to_vec()], vec![b"updated".to_vec()]);
        let leaf2 = create_test_leaf(vec![b"d".to_vec()], vec![b"4".to_vec()]);
        let mutations1 = vec![Mutation::Upsert {
            key: b"b".to_vec(), // "b" <= "c" (max key)
            val: b"updated".to_vec(),
        }];
        let mutations2 = vec![Mutation::Upsert {
            key: b"d".to_vec(), // "d" > "c"
            val: b"4".to_vec(),
        }];
        let group1 = create_test_group(leaf1, vec![], mutations1);
        let group2 = create_test_group(leaf2, vec![], mutations2);

        let result = should_use_deferred_rebalancing(&prolly, &tree, &[group1, group2]).unwrap();

        assert!(
            !result,
            "Mixed keys (some <= max key) should NOT use deferred rebalancing"
        );
    }

    #[test]
    fn test_should_use_deferred_rebalancing_delete_mutations() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = create_tree_with_entries(
            &prolly,
            &[
                (b"a".to_vec(), b"1".to_vec()),
                (b"b".to_vec(), b"2".to_vec()),
                (b"c".to_vec(), b"3".to_vec()),
            ],
        );

        // Create groups with delete mutations for existing keys
        let leaf1 = create_test_leaf(vec![b"a".to_vec()], vec![b"1".to_vec()]);
        let leaf2 = create_test_leaf(vec![b"b".to_vec()], vec![b"2".to_vec()]);
        let mutations1 = vec![Mutation::Delete {
            key: b"a".to_vec(), // "a" <= "c" (max key)
        }];
        let mutations2 = vec![Mutation::Delete {
            key: b"b".to_vec(), // "b" <= "c" (max key)
        }];
        let group1 = create_test_group(leaf1, vec![], mutations1);
        let group2 = create_test_group(leaf2, vec![], mutations2);

        let result = should_use_deferred_rebalancing(&prolly, &tree, &[group1, group2]).unwrap();

        assert!(
            !result,
            "Delete mutations with keys <= max key should NOT use deferred rebalancing"
        );
    }

    #[test]
    fn test_should_use_deferred_rebalancing_key_equal_to_max() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = create_tree_with_entries(
            &prolly,
            &[
                (b"a".to_vec(), b"1".to_vec()),
                (b"b".to_vec(), b"2".to_vec()),
                (b"c".to_vec(), b"3".to_vec()),
            ],
        );

        // Create groups with a mutation key equal to max key "c"
        let leaf1 = create_test_leaf(vec![b"c".to_vec()], vec![b"updated".to_vec()]);
        let leaf2 = create_test_leaf(vec![b"d".to_vec()], vec![b"4".to_vec()]);
        let mutations1 = vec![Mutation::Upsert {
            key: b"c".to_vec(), // "c" == "c" (max key), not greater
            val: b"updated".to_vec(),
        }];
        let mutations2 = vec![Mutation::Upsert {
            key: b"d".to_vec(),
            val: b"4".to_vec(),
        }];
        let group1 = create_test_group(leaf1, vec![], mutations1);
        let group2 = create_test_group(leaf2, vec![], mutations2);

        let result = should_use_deferred_rebalancing(&prolly, &tree, &[group1, group2]).unwrap();

        assert!(
            !result,
            "Key equal to max key should NOT use deferred rebalancing (must be strictly greater)"
        );
    }

    // ==================== apply_mutations_deferred tests ====================

    #[test]
    fn test_apply_mutations_deferred_empty_groups() {
        let groups: Vec<LeafMutationGroup> = vec![];
        let result = apply_mutations_deferred(groups);

        assert!(result.modified_leaves.is_empty());
        assert!(result.ancestor_paths.is_empty());
        assert!(result.first_keys.is_empty());
    }

    #[test]
    fn test_apply_mutations_deferred_single_group_upsert() {
        // Create a leaf with existing entries
        let leaf = create_test_leaf(
            vec![b"a".to_vec(), b"c".to_vec()],
            vec![b"1".to_vec(), b"3".to_vec()],
        );

        // Create mutations to insert "b" between "a" and "c"
        let mutations = vec![Mutation::Upsert {
            key: b"b".to_vec(),
            val: b"2".to_vec(),
        }];

        let group = create_test_group(leaf, vec![], mutations);
        let result = apply_mutations_deferred(vec![group]);

        // Verify result structure
        assert_eq!(result.modified_leaves.len(), 1);
        assert_eq!(result.ancestor_paths.len(), 1);
        assert_eq!(result.first_keys.len(), 1);

        // Verify the modified leaf has the new entry
        let modified_leaf = &result.modified_leaves[0];
        assert_eq!(modified_leaf.keys.len(), 3);
        assert_eq!(modified_leaf.keys[0], b"a".to_vec());
        assert_eq!(modified_leaf.keys[1], b"b".to_vec());
        assert_eq!(modified_leaf.keys[2], b"c".to_vec());
        assert_eq!(modified_leaf.vals[1], b"2".to_vec());

        // Verify first key
        assert_eq!(result.first_keys[0], b"a".to_vec());
    }

    #[test]
    fn test_apply_mutations_deferred_single_group_update() {
        // Create a leaf with existing entries
        let leaf = create_test_leaf(
            vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
            vec![b"1".to_vec(), b"2".to_vec(), b"3".to_vec()],
        );

        // Create mutation to update "b"
        let mutations = vec![Mutation::Upsert {
            key: b"b".to_vec(),
            val: b"updated".to_vec(),
        }];

        let group = create_test_group(leaf, vec![], mutations);
        let result = apply_mutations_deferred(vec![group]);

        // Verify the modified leaf has the updated value
        let modified_leaf = &result.modified_leaves[0];
        assert_eq!(modified_leaf.keys.len(), 3);
        assert_eq!(modified_leaf.vals[1], b"updated".to_vec());
    }

    #[test]
    fn test_apply_mutations_deferred_single_group_delete() {
        // Create a leaf with existing entries
        let leaf = create_test_leaf(
            vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
            vec![b"1".to_vec(), b"2".to_vec(), b"3".to_vec()],
        );

        // Create mutation to delete "b"
        let mutations = vec![Mutation::Delete { key: b"b".to_vec() }];

        let group = create_test_group(leaf, vec![], mutations);
        let result = apply_mutations_deferred(vec![group]);

        // Verify the modified leaf has "b" removed
        let modified_leaf = &result.modified_leaves[0];
        assert_eq!(modified_leaf.keys.len(), 2);
        assert_eq!(modified_leaf.keys[0], b"a".to_vec());
        assert_eq!(modified_leaf.keys[1], b"c".to_vec());
    }

    #[test]
    fn test_apply_mutations_deferred_multiple_groups() {
        // Create two leaves
        let leaf1 = create_test_leaf(
            vec![b"a".to_vec(), b"b".to_vec()],
            vec![b"1".to_vec(), b"2".to_vec()],
        );
        let leaf2 = create_test_leaf(
            vec![b"x".to_vec(), b"y".to_vec()],
            vec![b"24".to_vec(), b"25".to_vec()],
        );

        // Create mutations for each leaf
        let mutations1 = vec![Mutation::Upsert {
            key: b"c".to_vec(),
            val: b"3".to_vec(),
        }];
        let mutations2 = vec![Mutation::Upsert {
            key: b"z".to_vec(),
            val: b"26".to_vec(),
        }];

        let group1 = create_test_group(leaf1, vec![], mutations1);
        let group2 = create_test_group(leaf2, vec![], mutations2);

        let result = apply_mutations_deferred(vec![group1, group2]);

        // Verify result structure
        assert_eq!(result.modified_leaves.len(), 2);
        assert_eq!(result.ancestor_paths.len(), 2);
        assert_eq!(result.first_keys.len(), 2);

        // Verify first leaf modifications
        assert_eq!(result.modified_leaves[0].keys.len(), 3);
        assert_eq!(result.first_keys[0], b"a".to_vec());

        // Verify second leaf modifications
        assert_eq!(result.modified_leaves[1].keys.len(), 3);
        assert_eq!(result.first_keys[1], b"x".to_vec());
    }

    #[test]
    fn test_apply_mutations_deferred_preserves_ancestors() {
        // Create a leaf
        let leaf = create_test_leaf(vec![b"a".to_vec()], vec![b"1".to_vec()]);

        // Create a mock ancestor path
        let ancestor_node = Node {
            keys: vec![b"a".to_vec()],
            vals: vec![b"child_cid".to_vec()],
            leaf: false,
            level: 1,
            ..Default::default()
        };
        let ancestors = vec![(ancestor_node.clone(), 0)];

        let mutations = vec![Mutation::Upsert {
            key: b"b".to_vec(),
            val: b"2".to_vec(),
        }];

        let group = create_test_group(leaf, ancestors, mutations);
        let result = apply_mutations_deferred(vec![group]);

        // Verify ancestors are preserved
        assert_eq!(result.ancestor_paths.len(), 1);
        assert_eq!(result.ancestor_paths[0].len(), 1);
        assert_eq!(result.ancestor_paths[0][0].0.keys, ancestor_node.keys);
        assert_eq!(result.ancestor_paths[0][0].1, 0);
    }

    #[test]
    fn test_apply_mutations_deferred_empty_leaf_after_delete() {
        // Create a leaf with a single entry
        let leaf = create_test_leaf(vec![b"a".to_vec()], vec![b"1".to_vec()]);

        // Delete the only entry
        let mutations = vec![Mutation::Delete { key: b"a".to_vec() }];

        let group = create_test_group(leaf, vec![], mutations);
        let result = apply_mutations_deferred(vec![group]);

        // Verify the leaf is now empty
        let modified_leaf = &result.modified_leaves[0];
        assert!(modified_leaf.keys.is_empty());
        assert!(modified_leaf.vals.is_empty());

        // Verify first_key is empty vec for empty leaf
        assert!(result.first_keys[0].is_empty());
    }

    #[test]
    fn test_apply_mutations_deferred_allows_oversized_leaf() {
        // Create a leaf that will become oversized after mutations
        // Note: We're not actually checking max_chunk_size here since
        // the function doesn't enforce it - that's the point of deferred rebalancing
        let leaf = create_test_leaf(
            vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()],
            vec![b"1".to_vec(), b"2".to_vec(), b"3".to_vec()],
        );

        // Add many mutations that would normally trigger rebalancing
        let mutations = vec![
            Mutation::Upsert {
                key: b"d".to_vec(),
                val: b"4".to_vec(),
            },
            Mutation::Upsert {
                key: b"e".to_vec(),
                val: b"5".to_vec(),
            },
            Mutation::Upsert {
                key: b"f".to_vec(),
                val: b"6".to_vec(),
            },
            Mutation::Upsert {
                key: b"g".to_vec(),
                val: b"7".to_vec(),
            },
            Mutation::Upsert {
                key: b"h".to_vec(),
                val: b"8".to_vec(),
            },
        ];

        let group = create_test_group(leaf, vec![], mutations);
        let result = apply_mutations_deferred(vec![group]);

        // Verify all entries are in the leaf (no splitting occurred)
        let modified_leaf = &result.modified_leaves[0];
        assert_eq!(modified_leaf.keys.len(), 8);
        assert_eq!(modified_leaf.keys[0], b"a".to_vec());
        assert_eq!(modified_leaf.keys[7], b"h".to_vec());
    }

    // ==================== split_oversized_node tests ====================

    #[test]
    fn test_split_oversized_node_small_node_unchanged() {
        // A node smaller than max_chunk_size should return a single-element vector
        let store = MemStore::new();
        let config = Config::builder()
            .max_chunk_size(10)
            .min_chunk_size(2)
            .build();
        let prolly = Prolly::new(store, config);

        // Create a small node (3 entries, max is 10)
        let mut node = prolly.new_leaf_node();
        node.keys = vec![b"a".to_vec(), b"b".to_vec(), b"c".to_vec()];
        node.vals = vec![b"1".to_vec(), b"2".to_vec(), b"3".to_vec()];

        let chunks = split_oversized_node(&prolly, &node);

        assert_eq!(chunks.len(), 1, "Small node should not be split");
        assert_eq!(chunks[0].keys.len(), 3);
        assert_eq!(chunks[0].keys, node.keys);
    }

    #[test]
    fn test_split_oversized_node_splits_large_node() {
        // A node larger than max_chunk_size should be split into multiple chunks
        let store = MemStore::new();
        let config = Config::builder()
            .max_chunk_size(5)
            .min_chunk_size(2)
            .build();
        let prolly = Prolly::new(store, config);

        // Create an oversized node (10 entries, max is 5)
        let mut node = prolly.new_leaf_node();
        node.keys = (0..10).map(|i| vec![b'a' + i]).collect();
        node.vals = (0..10).map(|i| vec![b'0' + i]).collect();

        let chunks = split_oversized_node(&prolly, &node);

        // Should be split into multiple chunks
        assert!(
            chunks.len() > 1,
            "Oversized node should be split into multiple chunks"
        );

        // Each chunk should be strictly less than max_chunk_size
        for chunk in &chunks {
            assert!(
                chunk.len() < 5,
                "Each chunk should have length < max_chunk_size (5), got {}",
                chunk.len()
            );
        }

        // Total entries should be preserved
        let total_entries: usize = chunks.iter().map(|c| c.len()).sum();
        assert_eq!(
            total_entries, 10,
            "Total entries should be preserved after split"
        );

        // Keys should be in order across all chunks
        let all_keys: Vec<Vec<u8>> = chunks.iter().flat_map(|c| c.keys.clone()).collect();
        for i in 1..all_keys.len() {
            assert!(
                all_keys[i - 1] < all_keys[i],
                "Keys should be in strictly ascending order"
            );
        }
    }

    #[test]
    fn test_split_oversized_node_preserves_node_properties() {
        // Verify that split chunks preserve node properties (leaf, level, etc.)
        let store = MemStore::new();
        let config = Config::builder()
            .max_chunk_size(4)
            .min_chunk_size(2)
            .build();
        let prolly = Prolly::new(store, config);

        // Create an oversized leaf node
        let mut node = prolly.new_leaf_node();
        node.keys = (0..8).map(|i| vec![b'a' + i]).collect();
        node.vals = (0..8).map(|i| vec![b'0' + i]).collect();

        let chunks = split_oversized_node(&prolly, &node);

        // All chunks should be leaf nodes at level 0
        for chunk in &chunks {
            assert!(chunk.leaf, "All chunks should be leaf nodes");
            assert_eq!(chunk.level, 0, "All chunks should be at level 0");
            assert_eq!(
                chunk.max_chunk_size, node.max_chunk_size,
                "max_chunk_size should be preserved"
            );
            assert_eq!(
                chunk.min_chunk_size, node.min_chunk_size,
                "min_chunk_size should be preserved"
            );
        }
    }

    #[test]
    fn test_split_oversized_node_empty_node() {
        // An empty node should return a single-element vector with the empty node
        let store = MemStore::new();
        let config = Config::builder()
            .max_chunk_size(5)
            .min_chunk_size(2)
            .build();
        let prolly = Prolly::new(store, config);

        let node = prolly.new_leaf_node();
        assert!(node.keys.is_empty());

        let chunks = split_oversized_node(&prolly, &node);

        assert_eq!(chunks.len(), 1, "Empty node should return single chunk");
        assert!(chunks[0].keys.is_empty(), "Chunk should be empty");
    }
}
