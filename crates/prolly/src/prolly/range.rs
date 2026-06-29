//! Range iteration operations for Prolly trees
//!
//! This module handles range queries and iteration over key-value pairs
//! within specified bounds. It provides efficient traversal of the tree
//! in lexicographic order with support for start and end bounds.
//!
//! # Overview
//!
//! Range iteration allows traversing key-value pairs in sorted order within
//! a specified key range. The iterator efficiently navigates the tree structure,
//! handling node boundaries transparently.
//!
//! # Iteration Behavior
//!
//! ## Start Bound (Inclusive)
//!
//! The iterator begins at the first key greater than or equal to the start bound.
//! If the start key exists, iteration begins there. If not, iteration begins at
//! the next key in lexicographic order.
//!
//! Use an empty slice (`&[]`) to start from the beginning of the tree.
//!
//! ## End Bound (Exclusive)
//!
//! The iterator stops before reaching the end bound. Keys equal to or greater
//! than the end bound are not yielded.
//!
//! Use `None` to iterate to the end of the tree.
//!
//! # Implementation Details
//!
//! The iterator maintains a stack of (node, index) pairs representing the
//! current position in the tree. This allows efficient traversal across
//! node boundaries without restarting from the root.
//!
//! ## Traversal Algorithm
//!
//! 1. **Initial positioning**: Find the path to the start key
//! 2. **Leaf iteration**: Yield entries from the current leaf
//! 3. **Node advancement**: When a leaf is exhausted, backtrack to find the next leaf
//! 4. **Bound checking**: Stop when the end bound is reached
//!
//! # Example
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
//! tree = prolly.put(&tree, b"d".to_vec(), b"4".to_vec()).unwrap();
//!
//! // Iterate over all keys
//! for result in prolly.range(&tree, &[], None).unwrap() {
//!     let (key, val) = result.unwrap();
//!     println!("{:?} -> {:?}", key, val);
//! }
//!
//! // Iterate over range [b, d) - yields "b" and "c"
//! for result in prolly.range(&tree, b"b", Some(b"d")).unwrap() {
//!     let (key, val) = result.unwrap();
//!     println!("{:?} -> {:?}", key, val);
//! }
//! ```
//!
//! # Performance
//!
//! - **Initial seek**: O(log n) to find the starting position
//! - **Per-entry**: O(1) amortized for sequential access within a leaf
//! - **Node transitions**: O(log n) worst case, but typically O(1) amortized
//!
//! The iterator is lazy and only loads nodes as needed, making it memory-efficient
//! for large trees.

use super::cid::Cid;
use super::error::Error;
use super::node::Node;
use super::store::Store;
use super::tree::Tree;

use super::Prolly;
use std::sync::Arc;

type RangeItem = Result<(Vec<u8>, Vec<u8>), Error>;
pub(crate) const RANGE_CHILD_PREFETCH_PARALLELISM: usize = 16;

/// Create a range iterator over key-value pairs.
///
/// Returns an iterator that yields `(key, value)` pairs in lexicographic order,
/// starting from `start` (inclusive) up to `end` (exclusive).
///
/// # Arguments
/// * `prolly` - Reference to the Prolly tree manager
/// * `tree` - The tree to iterate over
/// * `start` - The starting key (inclusive). Use `&[]` to start from the beginning.
/// * `end` - Optional ending key (exclusive). Use `None` to iterate to the end.
///
/// # Returns
/// * `Ok(RangeIter)` - An iterator over the range
/// * `Err` on storage or deserialization errors during path finding
pub fn create_range_iter<'a, S: Store>(
    prolly: &'a Prolly<S>,
    tree: &Tree,
    start: &[u8],
    end: Option<&[u8]>,
) -> Result<RangeIter<'a, S>, Error> {
    if end.is_some_and(|end| end <= start) {
        return Ok(RangeIter::new(prolly, Vec::new(), start, end));
    }

    // Find path to start key
    let path = prolly.find_path_arcs(tree, start)?;
    Ok(RangeIter::new(prolly, path, start, end))
}

/// Iterator over key-value pairs in a range.
///
/// Created by [`Prolly::range`] or [`create_range_iter`]. Yields `(key, value)` pairs
/// in lexicographic order.
///
/// Maintains a stack of (node, index) pairs to track the current position in the tree
/// and supports optional end bounds for range queries.
pub struct RangeIter<'a, S: Store> {
    /// Reference to the Prolly tree manager
    prolly: &'a Prolly<S>,
    /// Stack of (node, index) pairs representing the traversal path
    stack: Vec<(Arc<Node>, usize)>,
    /// Optional end bound (exclusive)
    end: Option<Vec<u8>>,
    /// Whether we've started iteration (for positioning at start key)
    started: bool,
    /// The start key for initial positioning
    start_key: Vec<u8>,
}

impl<'a, S: Store> RangeIter<'a, S> {
    /// Create a new range iterator.
    ///
    /// # Arguments
    /// * `prolly` - Reference to the Prolly tree manager
    /// * `stack` - Initial traversal path from find_path
    /// * `start` - The starting key (inclusive)
    /// * `end` - Optional ending key (exclusive)
    pub(crate) fn new(
        prolly: &'a Prolly<S>,
        stack: Vec<(Arc<Node>, usize)>,
        start: &[u8],
        end: Option<&[u8]>,
    ) -> Self {
        Self {
            prolly,
            stack,
            end: end.map(|e| e.to_vec()),
            started: false,
            start_key: start.to_vec(),
        }
    }

    /// Position the iterator at the first key >= start_key.
    ///
    /// Called on the first iteration to find the correct starting position
    /// in the leaf node.
    fn position_at_start(&mut self) -> Option<RangeItem> {
        self.started = true;

        // If stack is empty, tree is empty
        if self.stack.is_empty() {
            return None;
        }

        // Find the first key >= start_key in the leaf
        let (node, idx) = self.stack.last_mut()?;

        // If we're at a leaf, find the correct starting position
        if node.leaf {
            // Find first key >= start_key
            let start_idx = match node.search(&self.start_key) {
                Ok(i) => i,  // Exact match
                Err(i) => i, // First key > start_key
            };

            *idx = start_idx;

            // Check if we're past the end of this node
            if *idx >= node.len() {
                // Need to advance to next node
                return self.advance_to_next_leaf();
            }

            match leaf_entry_before_end(node, *idx, self.end.as_deref()) {
                Ok(Some(entry)) => {
                    *idx += 1;
                    return Some(Ok(entry));
                }
                Ok(None) => return None,
                Err(e) => return Some(Err(e)),
            }
        }

        // Not at a leaf - descend to the correct leaf
        self.descend_to_leaf()
    }

    /// Descend from current internal node to the leftmost leaf.
    ///
    /// Follows child pointers until reaching a leaf node, then returns
    /// the first entry from that leaf.
    fn descend_to_leaf(&mut self) -> Option<RangeItem> {
        loop {
            let (node, idx) = self.stack.last()?;

            if node.leaf {
                // We're at a leaf, return the current entry
                if *idx >= node.len() {
                    return self.advance_to_next_leaf();
                }

                match leaf_entry_before_end(node, *idx, self.end.as_deref()) {
                    Ok(Some(entry)) => {
                        if let Some((_, idx)) = self.stack.last_mut() {
                            *idx += 1;
                        }
                        return Some(Ok(entry));
                    }
                    Ok(None) => return None,
                    Err(e) => return Some(Err(e)),
                }
            }

            // Internal node - descend to child
            if *idx >= node.len() {
                // No more children, go back up
                return self.advance_to_next_leaf();
            }

            match child_starts_at_or_after_end(self.end.as_deref(), node, *idx) {
                Ok(true) => return None,
                Ok(false) => {}
                Err(e) => return Some(Err(e)),
            }

            match self.load_child_for_descent(node, *idx) {
                Ok(child) => {
                    self.stack.push((child, 0));
                }
                Err(e) => return Some(Err(e)),
            }
        }
    }

    /// Advance to the next leaf node when current leaf is exhausted.
    ///
    /// Pops nodes from the stack until finding a parent with more children,
    /// then descends to the next child's leftmost leaf.
    fn advance_to_next_leaf(&mut self) -> Option<RangeItem> {
        loop {
            // Pop the current node
            self.stack.pop();

            if self.stack.is_empty() {
                return None;
            }

            // Increment parent's index
            if let Some((parent, parent_idx)) = self.stack.last_mut() {
                *parent_idx += 1;

                // Check if parent has more children
                if *parent_idx < parent.len() {
                    match child_starts_at_or_after_end(self.end.as_deref(), parent, *parent_idx) {
                        Ok(true) => return None,
                        Ok(false) => {}
                        Err(e) => return Some(Err(e)),
                    }
                    // Descend to the next child
                    return self.descend_to_leaf();
                }
                if parent.keys.len() != parent.vals.len() {
                    return Some(Err(Error::InvalidNode));
                }
                // Otherwise, continue popping
            }
        }
    }

    fn load_child_for_descent(&self, node: &Node, idx: usize) -> Result<Arc<Node>, Error> {
        let child_cid = child_cid_at(node, idx)?;

        if !self.prolly.store().prefers_batch_reads() {
            return self.prolly.load_arc(&child_cid);
        }

        if let Some(child) = self.prolly.cached_node_arc(&child_cid) {
            return Ok(child);
        }

        let max_child_idx = node
            .len()
            .min(idx.saturating_add(RANGE_CHILD_PREFETCH_PARALLELISM));
        let mut child_cids = Vec::with_capacity(max_child_idx.saturating_sub(idx));
        child_cids.push(child_cid);

        for child_idx in idx + 1..max_child_idx {
            match child_starts_at_or_after_end(self.end.as_deref(), node, child_idx) {
                Ok(true) => break,
                Ok(false) => {}
                Err(_) => break,
            }

            match child_cid_at(node, child_idx) {
                Ok(cid) => child_cids.push(cid),
                Err(_) => break,
            }
        }

        if child_cids.len() == 1 {
            return self.prolly.load_arc(&child_cids[0]);
        }

        let children = self
            .prolly
            .load_many_ordered_with_parallelism(&child_cids, RANGE_CHILD_PREFETCH_PARALLELISM)?;
        children.into_iter().next().ok_or(Error::InvalidNode)
    }
}

/// Iterator implementation for RangeIter.
///
/// Yields `(key, value)` pairs in lexicographic order, handling cursor advancement
/// across node boundaries and checking end bounds for each yielded entry.
impl<'a, S: Store> Iterator for RangeIter<'a, S> {
    type Item = Result<(Vec<u8>, Vec<u8>), Error>;

    fn next(&mut self) -> Option<Self::Item> {
        // First call: position at start key
        if !self.started {
            return self.position_at_start();
        }

        loop {
            let (node, idx) = self.stack.last_mut()?;

            if !node.leaf && node.keys.len() != node.vals.len() {
                return Some(Err(Error::InvalidNode));
            }

            // If we've exhausted this node, advance to next
            if *idx >= node.len() {
                match self.advance_to_next_leaf() {
                    Some(result) => return Some(result),
                    None => return None,
                }
            }

            // If we're at a leaf, yield the current entry
            if node.leaf {
                match leaf_entry_before_end(node, *idx, self.end.as_deref()) {
                    Ok(Some(entry)) => {
                        *idx += 1;
                        return Some(Ok(entry));
                    }
                    Ok(None) => return None,
                    Err(e) => return Some(Err(e)),
                }
            }

            // Internal node - descend to child
            match child_starts_at_or_after_end(self.end.as_deref(), node, *idx) {
                Ok(true) => return None,
                Ok(false) => {}
                Err(e) => return Some(Err(e)),
            }

            let child = {
                let (node, idx) = self.stack.last()?;
                self.load_child_for_descent(node, *idx)
            };

            match child {
                Ok(child) => {
                    self.stack.push((child, 0));
                }
                Err(e) => return Some(Err(e)),
            }
        }
    }
}

fn leaf_entry_before_end(
    node: &Node,
    idx: usize,
    end: Option<&[u8]>,
) -> Result<Option<(Vec<u8>, Vec<u8>)>, Error> {
    let key = node.keys.get(idx).ok_or(Error::InvalidNode)?;
    if let Some(end) = end {
        if key.as_slice() >= end {
            return Ok(None);
        }
    }

    let val = node.vals.get(idx).ok_or(Error::InvalidNode)?;
    Ok(Some((key.clone(), val.clone())))
}

fn child_starts_at_or_after_end(
    end: Option<&[u8]>,
    node: &Node,
    child_index: usize,
) -> Result<bool, Error> {
    let Some(end) = end else {
        return Ok(false);
    };

    let first_key = node.keys.get(child_index).ok_or(Error::InvalidNode)?;
    Ok(first_key.as_slice() >= end)
}

fn child_cid_at(node: &Node, idx: usize) -> Result<Cid, Error> {
    let child = node.vals.get(idx).ok_or(Error::InvalidNode)?;
    Ok(Cid(child
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidNode)?))
}
