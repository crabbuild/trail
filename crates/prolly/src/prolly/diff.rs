//! Diff and merge operations for Prolly trees
//!
//! This module handles computing differences between trees and performing
//! three-way merges with conflict resolution. These operations enable
//! version control semantics for Prolly trees.
//!
//! # Overview
//!
//! Prolly trees are well-suited for version control because their content-addressed
//! structure enables efficient comparison. Two trees with the same root CID are
//! identical, and trees with different roots can be compared by examining their
//! differences.
//!
//! # Diff Operations
//!
//! The [`compute_diff`] function compares two trees and returns a list of differences:
//!
//! - **Added**: Keys that exist in the `other` tree but not in `base`
//! - **Removed**: Keys that exist in `base` but not in `other`
//! - **Changed**: Keys that exist in both trees but with different values
//!
//! ## Short-Circuit Optimization
//!
//! If both trees have the same root CID, the diff returns immediately with an
//! empty result. This makes comparing identical trees O(1).
//!
//! ## Example
//!
//! ```rust
//! use prolly::{Prolly, MemStore, Config, Diff};
//!
//! let store = MemStore::new();
//! let prolly = Prolly::new(store, Config::default());
//!
//! let base = prolly.create();
//! let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
//!
//! let modified = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
//!
//! let diffs = prolly.diff(&base, &modified).unwrap();
//! // diffs contains: Added { key: "b", val: "2" }
//! ```
//!
//! # Merge Operations
//!
//! The [`merge_trees`] function performs a three-way merge using a common ancestor (base).
//! This is the standard merge algorithm used in version control systems.
//!
//! ## Three-Way Merge Algorithm
//!
//! 1. Return immediately when one branch is unchanged or both branches match
//! 2. Compute the diff from `base` to `right`
//! 3. Batch-lookup the right-changed keys in `left`
//! 4. Apply changes from `right` that don't conflict with `left`
//! 5. Detect and handle conflicts
//!
//! ## Conflict Detection
//!
//! A conflict occurs when both branches modify the same key differently from the base:
//!
//! - Both branches change the same key to different values
//! - One branch changes a key while the other deletes it
//! - Both branches add the same key with different values
//!
//! ## Conflict Resolution
//!
//! When a conflict is detected:
//!
//! 1. If a resolver function is provided, it's called with conflict details
//! 2. If the resolver returns `Some(value)`, that value is used
//! 3. If the resolver returns `None` or no resolver is provided, an error is returned
//!
//! ## Example
//!
//! ```rust
//! use prolly::{Prolly, MemStore, Config, Conflict, Resolver};
//!
//! let store = MemStore::new();
//! let prolly = Prolly::new(store, Config::default());
//!
//! // Create base and divergent branches
//! let base = prolly.create();
//! let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
//!
//! let left = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
//! let right = prolly.put(&base, b"c".to_vec(), b"3".to_vec()).unwrap();
//!
//! // Merge without conflicts
//! let merged = prolly.merge(&base, &left, &right, None).unwrap();
//!
//! // With conflict resolution
//! let resolver: Resolver = Box::new(|conflict| Some(conflict.left.clone()));
//! let merged = prolly.merge(&base, &left, &right, Some(resolver)).unwrap();
//! ```
//!
//! # Performance Considerations
//!
//! - **Same root**: O(1) - immediate return for identical trees
//! - **Different roots**: O(changed subtrees) when chunk boundaries align, with
//!   a local full-scan fallback when boundaries diverge

use std::collections::{BTreeMap, HashSet, VecDeque};

use super::batch::{get_max_key, BatchWriteCollector};
use super::cid::Cid;
use super::error::{Conflict, Diff, Error, Mutation, Resolver};
use super::node::Node;
use super::store::Store;
use super::tree::Tree;

use super::Prolly;

type ChildSpanCid<'a> = (Option<&'a [u8]>, Cid);

#[derive(Clone, Debug, PartialEq)]
struct MergeChange {
    base: Option<Vec<u8>>,
    value: Option<Vec<u8>>,
}

enum DiffFrame {
    Compare {
        base_cid: Cid,
        other_cid: Cid,
        span_end: Option<Vec<u8>>,
    },
    Added {
        cid: Cid,
    },
    Removed {
        cid: Cid,
    },
}

/// Iterator over tree differences that preserves the subtree-pruning behavior
/// of eager diff without collecting the whole result upfront.
pub(crate) struct StructuralDiffIter<'a, S: Store> {
    prolly: &'a Prolly<S>,
    stack: Vec<DiffFrame>,
    pending: VecDeque<Diff>,
    failed: bool,
}

impl<'a, S: Store> StructuralDiffIter<'a, S> {
    fn new(prolly: &'a Prolly<S>, base: &Tree, other: &Tree) -> Self {
        let stack = match (&base.root, &other.root) {
            (Some(base_cid), Some(other_cid)) if base_cid != other_cid => {
                vec![DiffFrame::Compare {
                    base_cid: base_cid.clone(),
                    other_cid: other_cid.clone(),
                    span_end: None,
                }]
            }
            (Some(base_cid), None) => vec![DiffFrame::Removed {
                cid: base_cid.clone(),
            }],
            (None, Some(other_cid)) => vec![DiffFrame::Added {
                cid: other_cid.clone(),
            }],
            _ => Vec::new(),
        };

        Self {
            prolly,
            stack,
            pending: VecDeque::new(),
            failed: false,
        }
    }

    fn fill_pending(&mut self) -> Result<(), Error> {
        while self.pending.is_empty() {
            let Some(frame) = self.stack.pop() else {
                return Ok(());
            };

            match frame {
                DiffFrame::Compare {
                    base_cid,
                    other_cid,
                    span_end,
                } => self.process_compare(base_cid, other_cid, span_end.as_deref())?,
                DiffFrame::Added { cid } => self.process_added(cid)?,
                DiffFrame::Removed { cid } => self.process_removed(cid)?,
            }
        }

        Ok(())
    }

    fn process_compare(
        &mut self,
        base_cid: Cid,
        other_cid: Cid,
        span_end: Option<&[u8]>,
    ) -> Result<(), Error> {
        if base_cid == other_cid {
            return Ok(());
        }

        let nodes = self
            .prolly
            .load_many_ordered(&[base_cid.clone(), other_cid.clone()])?;
        let base = nodes[0].clone();
        let other = nodes[1].clone();

        match (base.leaf, other.leaf) {
            (true, true) => {
                let mut diffs = Vec::new();
                diff_leaf_nodes(&base, &other, &mut diffs)?;
                self.pending.extend(diffs);
            }
            (false, false) if base.level == other.level => {
                self.enqueue_internal_diff(&base, &other, span_end)?;
            }
            _ => {
                let mut diffs = Vec::new();
                diff_collected_nodes(self.prolly, &base, &other, &mut diffs)?;
                self.pending.extend(diffs);
            }
        }

        Ok(())
    }

    fn enqueue_internal_diff(
        &mut self,
        base: &Node,
        other: &Node,
        span_end: Option<&[u8]>,
    ) -> Result<(), Error> {
        let mut frames = Vec::new();
        let mut base_idx = 0;
        let mut other_idx = 0;

        while base_idx < base.len() && other_idx < other.len() {
            let base_start = base.keys[base_idx].as_slice();
            let other_start = other.keys[other_idx].as_slice();
            let base_end = child_span_end(base, base_idx, span_end);
            let other_end = child_span_end(other, other_idx, span_end);

            if base_start == other_start && base_end == other_end {
                let base_cid = child_cid(base, base_idx)?;
                let other_cid = child_cid(other, other_idx)?;
                if base_cid != other_cid {
                    frames.push(DiffFrame::Compare {
                        base_cid,
                        other_cid,
                        span_end: base_end.map(<[u8]>::to_vec),
                    });
                }
                base_idx += 1;
                other_idx += 1;
            } else if span_ends_before_or_at(base_end, other_start) {
                frames.push(DiffFrame::Removed {
                    cid: child_cid(base, base_idx)?,
                });
                base_idx += 1;
            } else if span_ends_before_or_at(other_end, base_start) {
                frames.push(DiffFrame::Added {
                    cid: child_cid(other, other_idx)?,
                });
                other_idx += 1;
            } else {
                let mut diffs = Vec::new();
                diff_collected_nodes(self.prolly, base, other, &mut diffs)?;
                self.pending.extend(diffs);
                return Ok(());
            }
        }

        while base_idx < base.len() {
            frames.push(DiffFrame::Removed {
                cid: child_cid(base, base_idx)?,
            });
            base_idx += 1;
        }

        while other_idx < other.len() {
            frames.push(DiffFrame::Added {
                cid: child_cid(other, other_idx)?,
            });
            other_idx += 1;
        }

        self.prefetch_frame_roots(&frames)?;
        self.stack.extend(frames.into_iter().rev());
        Ok(())
    }

    fn process_added(&mut self, cid: Cid) -> Result<(), Error> {
        let node = self.prolly.load_arc(&cid)?;
        if node.leaf {
            self.pending.extend(
                node.keys
                    .iter()
                    .zip(&node.vals)
                    .map(|(key, val)| Diff::Added {
                        key: key.clone(),
                        val: val.clone(),
                    }),
            );
        } else {
            let mut frames = child_cids(&node)?
                .into_iter()
                .map(|cid| DiffFrame::Added { cid })
                .collect::<Vec<_>>();
            self.prefetch_frame_roots(&frames)?;
            frames.reverse();
            self.stack.extend(frames);
        }

        Ok(())
    }

    fn process_removed(&mut self, cid: Cid) -> Result<(), Error> {
        let node = self.prolly.load_arc(&cid)?;
        if node.leaf {
            self.pending.extend(
                node.keys
                    .iter()
                    .zip(&node.vals)
                    .map(|(key, val)| Diff::Removed {
                        key: key.clone(),
                        val: val.clone(),
                    }),
            );
        } else {
            let mut frames = child_cids(&node)?
                .into_iter()
                .map(|cid| DiffFrame::Removed { cid })
                .collect::<Vec<_>>();
            self.prefetch_frame_roots(&frames)?;
            frames.reverse();
            self.stack.extend(frames);
        }

        Ok(())
    }

    fn prefetch_frame_roots(&self, frames: &[DiffFrame]) -> Result<(), Error> {
        if frames.len() <= 1 || !self.prolly.store().prefers_batch_reads() {
            return Ok(());
        }

        let mut seen = HashSet::new();
        let mut cids = Vec::new();
        for frame in frames {
            match frame {
                DiffFrame::Compare {
                    base_cid,
                    other_cid,
                    ..
                } => {
                    if seen.insert(base_cid.clone()) {
                        cids.push(base_cid.clone());
                    }
                    if seen.insert(other_cid.clone()) {
                        cids.push(other_cid.clone());
                    }
                }
                DiffFrame::Added { cid } | DiffFrame::Removed { cid } => {
                    if seen.insert(cid.clone()) {
                        cids.push(cid.clone());
                    }
                }
            }
        }

        if !cids.is_empty() {
            let _ = self.prolly.load_many_ordered(&cids)?;
        }

        Ok(())
    }
}

impl<S: Store> Iterator for StructuralDiffIter<'_, S> {
    type Item = Result<Diff, Error>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }

        if let Err(err) = self.fill_pending() {
            self.failed = true;
            self.stack.clear();
            self.pending.clear();
            return Some(Err(err));
        }

        self.pending.pop_front().map(Ok)
    }
}

pub(crate) fn stream_diff<'a, S: Store>(
    prolly: &'a Prolly<S>,
    base: &Tree,
    other: &Tree,
) -> StructuralDiffIter<'a, S> {
    StructuralDiffIter::new(prolly, base, other)
}

/// Compute the difference between two trees.
///
/// Returns a vector of `Diff` entries representing the changes needed to
/// transform `base` into `other`. Yields Added entries for keys that exist in
/// other but not in base, Changed entries for keys with different values, and
/// Removed entries for keys that exist in base but not in other.
///
/// # Arguments
/// * `prolly` - The Prolly tree manager
/// * `base` - The base tree to compare from
/// * `other` - The other tree to compare to
///
/// # Returns
/// * `Ok(Vec<Diff>)` - A vector of differences
/// * `Err` on storage or deserialization errors
///
/// # Short-circuit
/// If both trees have the same root CID, returns an empty vector immediately.
pub fn compute_diff<S: Store>(
    prolly: &Prolly<S>,
    base: &Tree,
    other: &Tree,
) -> Result<Vec<Diff>, Error> {
    if let Some(diffs) = try_append_only_diff(prolly, base, other)? {
        return Ok(diffs);
    }

    let mut diffs = Vec::new();
    for diff in stream_diff(prolly, base, other) {
        diffs.push(diff?);
    }

    Ok(diffs)
}

/// Compute the difference between two trees within the half-open key range
/// `[start, end)`.
///
/// This mirrors [`compute_diff`] but prunes whole child spans that cannot
/// overlap the requested range, so narrow range diffs do not have to
/// materialize both sides of the range first.
pub fn compute_range_diff<S: Store>(
    prolly: &Prolly<S>,
    base: &Tree,
    other: &Tree,
    start: &[u8],
    end: Option<&[u8]>,
) -> Result<Vec<Diff>, Error> {
    if end.is_some_and(|end| end <= start) || base.root == other.root {
        return Ok(Vec::new());
    }

    let mut diffs = Vec::new();

    match (&base.root, &other.root) {
        (Some(base_cid), Some(other_cid)) => {
            diff_range_nodes(prolly, base_cid, other_cid, None, start, end, &mut diffs)?;
        }
        (Some(base_cid), None) => {
            collect_removed_range_from_cid(prolly, base_cid, None, start, end, &mut diffs)?;
        }
        (None, Some(other_cid)) => {
            collect_added_range_from_cid(prolly, other_cid, None, start, end, &mut diffs)?;
        }
        (None, None) => {}
    }

    Ok(diffs)
}

fn diff_range_nodes<S: Store>(
    prolly: &Prolly<S>,
    base_cid: &Cid,
    other_cid: &Cid,
    span_end: Option<&[u8]>,
    range_start: &[u8],
    range_end: Option<&[u8]>,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    if base_cid == other_cid {
        return Ok(());
    }

    let base_node = prolly.load_arc(base_cid)?;
    let other_node = prolly.load_arc(other_cid)?;

    match (base_node.leaf, other_node.leaf) {
        (true, true) => {
            diff_leaf_nodes_range(&base_node, &other_node, range_start, range_end, diffs)
        }
        (false, false) if base_node.level == other_node.level => diff_internal_nodes_range(
            prolly,
            &base_node,
            &other_node,
            span_end,
            range_start,
            range_end,
            diffs,
        ),
        _ => diff_collected_nodes_range(
            prolly,
            &base_node,
            &other_node,
            span_end,
            range_start,
            range_end,
            diffs,
        ),
    }
}

fn diff_internal_nodes_range<S: Store>(
    prolly: &Prolly<S>,
    base: &Node,
    other: &Node,
    span_end: Option<&[u8]>,
    range_start: &[u8],
    range_end: Option<&[u8]>,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    let mut base_idx = 0;
    let mut other_idx = 0;

    while base_idx < base.len() && other_idx < other.len() {
        let base_start = base.keys[base_idx].as_slice();
        let other_start = other.keys[other_idx].as_slice();
        let base_end = child_span_end(base, base_idx, span_end);
        let other_end = child_span_end(other, other_idx, span_end);

        if span_ends_before_or_at(base_end, range_start) {
            base_idx += 1;
            continue;
        }
        if span_ends_before_or_at(other_end, range_start) {
            other_idx += 1;
            continue;
        }
        if range_ends_before_or_at(range_end, base_start)
            && range_ends_before_or_at(range_end, other_start)
        {
            break;
        }

        if base_start == other_start && base_end == other_end {
            if span_overlaps_range(base_start, base_end, range_start, range_end) {
                let base_cid = child_cid(base, base_idx)?;
                let other_cid = child_cid(other, other_idx)?;
                diff_range_nodes(
                    prolly,
                    &base_cid,
                    &other_cid,
                    base_end,
                    range_start,
                    range_end,
                    diffs,
                )?;
            }
            base_idx += 1;
            other_idx += 1;
        } else if span_ends_before_or_at(base_end, other_start) {
            if span_overlaps_range(base_start, base_end, range_start, range_end) {
                let base_cid = child_cid(base, base_idx)?;
                collect_removed_range_from_cid(
                    prolly,
                    &base_cid,
                    base_end,
                    range_start,
                    range_end,
                    diffs,
                )?;
            }
            base_idx += 1;
        } else if span_ends_before_or_at(other_end, base_start) {
            if span_overlaps_range(other_start, other_end, range_start, range_end) {
                let other_cid = child_cid(other, other_idx)?;
                collect_added_range_from_cid(
                    prolly,
                    &other_cid,
                    other_end,
                    range_start,
                    range_end,
                    diffs,
                )?;
            }
            other_idx += 1;
        } else {
            // Boundaries overlap without lining up. Keep the fallback local to
            // this subtree, but filter and prune by the requested range.
            return diff_collected_nodes_range(
                prolly,
                base,
                other,
                span_end,
                range_start,
                range_end,
                diffs,
            );
        }
    }

    while base_idx < base.len() {
        let base_start = base.keys[base_idx].as_slice();
        let base_end = child_span_end(base, base_idx, span_end);
        if span_overlaps_range(base_start, base_end, range_start, range_end) {
            let base_cid = child_cid(base, base_idx)?;
            collect_removed_range_from_cid(
                prolly,
                &base_cid,
                base_end,
                range_start,
                range_end,
                diffs,
            )?;
        } else if range_ends_before_or_at(range_end, base_start) {
            break;
        }
        base_idx += 1;
    }

    while other_idx < other.len() {
        let other_start = other.keys[other_idx].as_slice();
        let other_end = child_span_end(other, other_idx, span_end);
        if span_overlaps_range(other_start, other_end, range_start, range_end) {
            let other_cid = child_cid(other, other_idx)?;
            collect_added_range_from_cid(
                prolly,
                &other_cid,
                other_end,
                range_start,
                range_end,
                diffs,
            )?;
        } else if range_ends_before_or_at(range_end, other_start) {
            break;
        }
        other_idx += 1;
    }

    Ok(())
}

fn diff_leaf_nodes(base: &Node, other: &Node, diffs: &mut Vec<Diff>) -> Result<(), Error> {
    let mut base_idx = 0;
    let mut other_idx = 0;

    while base_idx < base.len() && other_idx < other.len() {
        let base_key = &base.keys[base_idx];
        let other_key = &other.keys[other_idx];

        match base_key.cmp(other_key) {
            std::cmp::Ordering::Less => {
                diffs.push(Diff::Removed {
                    key: base_key.clone(),
                    val: base.vals[base_idx].clone(),
                });
                base_idx += 1;
            }
            std::cmp::Ordering::Greater => {
                diffs.push(Diff::Added {
                    key: other_key.clone(),
                    val: other.vals[other_idx].clone(),
                });
                other_idx += 1;
            }
            std::cmp::Ordering::Equal => {
                let old = &base.vals[base_idx];
                let new = &other.vals[other_idx];
                if old != new {
                    diffs.push(Diff::Changed {
                        key: base_key.clone(),
                        old: old.clone(),
                        new: new.clone(),
                    });
                }
                base_idx += 1;
                other_idx += 1;
            }
        }
    }

    while base_idx < base.len() {
        diffs.push(Diff::Removed {
            key: base.keys[base_idx].clone(),
            val: base.vals[base_idx].clone(),
        });
        base_idx += 1;
    }

    while other_idx < other.len() {
        diffs.push(Diff::Added {
            key: other.keys[other_idx].clone(),
            val: other.vals[other_idx].clone(),
        });
        other_idx += 1;
    }

    Ok(())
}

fn diff_leaf_nodes_range(
    base: &Node,
    other: &Node,
    range_start: &[u8],
    range_end: Option<&[u8]>,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    let mut base_idx = lower_bound(&base.keys, range_start);
    let mut other_idx = lower_bound(&other.keys, range_start);

    while base_idx < base.len()
        && other_idx < other.len()
        && key_in_range(&base.keys[base_idx], range_start, range_end)
        && key_in_range(&other.keys[other_idx], range_start, range_end)
    {
        let base_key = &base.keys[base_idx];
        let other_key = &other.keys[other_idx];

        match base_key.cmp(other_key) {
            std::cmp::Ordering::Less => {
                diffs.push(Diff::Removed {
                    key: base_key.clone(),
                    val: base.vals[base_idx].clone(),
                });
                base_idx += 1;
            }
            std::cmp::Ordering::Greater => {
                diffs.push(Diff::Added {
                    key: other_key.clone(),
                    val: other.vals[other_idx].clone(),
                });
                other_idx += 1;
            }
            std::cmp::Ordering::Equal => {
                let old = &base.vals[base_idx];
                let new = &other.vals[other_idx];
                if old != new {
                    diffs.push(Diff::Changed {
                        key: base_key.clone(),
                        old: old.clone(),
                        new: new.clone(),
                    });
                }
                base_idx += 1;
                other_idx += 1;
            }
        }
    }

    while base_idx < base.len() && key_in_range(&base.keys[base_idx], range_start, range_end) {
        diffs.push(Diff::Removed {
            key: base.keys[base_idx].clone(),
            val: base.vals[base_idx].clone(),
        });
        base_idx += 1;
    }

    while other_idx < other.len() && key_in_range(&other.keys[other_idx], range_start, range_end) {
        diffs.push(Diff::Added {
            key: other.keys[other_idx].clone(),
            val: other.vals[other_idx].clone(),
        });
        other_idx += 1;
    }

    Ok(())
}

fn diff_collected_nodes<S: Store>(
    prolly: &Prolly<S>,
    base: &Node,
    other: &Node,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    let mut base_entries = Vec::new();
    let mut other_entries = Vec::new();
    collect_entries_from_node(prolly, base, &mut base_entries)?;
    collect_entries_from_node(prolly, other, &mut other_entries)?;
    diff_entry_slices(&base_entries, &other_entries, diffs);
    Ok(())
}

fn diff_collected_nodes_range<S: Store>(
    prolly: &Prolly<S>,
    base: &Node,
    other: &Node,
    span_end: Option<&[u8]>,
    range_start: &[u8],
    range_end: Option<&[u8]>,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    let mut base_entries = Vec::new();
    let mut other_entries = Vec::new();
    collect_entries_range_from_node(
        prolly,
        base,
        span_end,
        range_start,
        range_end,
        &mut base_entries,
    )?;
    collect_entries_range_from_node(
        prolly,
        other,
        span_end,
        range_start,
        range_end,
        &mut other_entries,
    )?;
    diff_entry_slices(&base_entries, &other_entries, diffs);
    Ok(())
}

fn diff_entry_slices(
    base: &[(Vec<u8>, Vec<u8>)],
    other: &[(Vec<u8>, Vec<u8>)],
    diffs: &mut Vec<Diff>,
) {
    let mut base_idx = 0;
    let mut other_idx = 0;

    while base_idx < base.len() && other_idx < other.len() {
        let (base_key, base_val) = &base[base_idx];
        let (other_key, other_val) = &other[other_idx];

        match base_key.cmp(other_key) {
            std::cmp::Ordering::Less => {
                diffs.push(Diff::Removed {
                    key: base_key.clone(),
                    val: base_val.clone(),
                });
                base_idx += 1;
            }
            std::cmp::Ordering::Greater => {
                diffs.push(Diff::Added {
                    key: other_key.clone(),
                    val: other_val.clone(),
                });
                other_idx += 1;
            }
            std::cmp::Ordering::Equal => {
                if base_val != other_val {
                    diffs.push(Diff::Changed {
                        key: base_key.clone(),
                        old: base_val.clone(),
                        new: other_val.clone(),
                    });
                }
                base_idx += 1;
                other_idx += 1;
            }
        }
    }

    for (key, val) in &base[base_idx..] {
        diffs.push(Diff::Removed {
            key: key.clone(),
            val: val.clone(),
        });
    }

    for (key, val) in &other[other_idx..] {
        diffs.push(Diff::Added {
            key: key.clone(),
            val: val.clone(),
        });
    }
}

fn collect_entries_from_node<S: Store>(
    prolly: &Prolly<S>,
    node: &Node,
    entries: &mut Vec<(Vec<u8>, Vec<u8>)>,
) -> Result<(), Error> {
    if node.leaf {
        entries.extend(node.keys.iter().cloned().zip(node.vals.iter().cloned()));
        return Ok(());
    }

    let child_cids = child_cids(node)?;
    for child_node in prolly.load_many_ordered(&child_cids)? {
        collect_entries_from_node(prolly, &child_node, entries)?;
    }

    Ok(())
}

fn collect_entries_range_from_node<S: Store>(
    prolly: &Prolly<S>,
    node: &Node,
    span_end: Option<&[u8]>,
    range_start: &[u8],
    range_end: Option<&[u8]>,
    entries: &mut Vec<(Vec<u8>, Vec<u8>)>,
) -> Result<(), Error> {
    if node.leaf {
        let mut idx = lower_bound(&node.keys, range_start);
        while idx < node.len() && key_in_range(&node.keys[idx], range_start, range_end) {
            entries.push((node.keys[idx].clone(), node.vals[idx].clone()));
            idx += 1;
        }
        return Ok(());
    }

    let child_spans = overlapping_child_cids(node, span_end, range_start, range_end)?;
    let child_cids = child_spans
        .iter()
        .map(|(_, cid)| cid.clone())
        .collect::<Vec<_>>();
    for ((child_end, _), child_node) in child_spans
        .into_iter()
        .zip(prolly.load_many_ordered(&child_cids)?)
    {
        collect_entries_range_from_node(
            prolly,
            &child_node,
            child_end,
            range_start,
            range_end,
            entries,
        )?;
    }

    Ok(())
}

fn collect_added_from_cid<S: Store>(
    prolly: &Prolly<S>,
    cid: &Cid,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    let node = prolly.load_arc(cid)?;
    collect_added_from_node(prolly, &node, diffs)
}

fn collect_added_from_node<S: Store>(
    prolly: &Prolly<S>,
    node: &Node,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    if node.leaf {
        for (key, val) in node.keys.iter().zip(&node.vals) {
            diffs.push(Diff::Added {
                key: key.clone(),
                val: val.clone(),
            });
        }
        return Ok(());
    }

    let child_cids = child_cids(node)?;
    for child_node in prolly.load_many_ordered(&child_cids)? {
        collect_added_from_node(prolly, &child_node, diffs)?;
    }

    Ok(())
}

fn collect_added_range_from_cid<S: Store>(
    prolly: &Prolly<S>,
    cid: &Cid,
    span_end: Option<&[u8]>,
    range_start: &[u8],
    range_end: Option<&[u8]>,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    let node = prolly.load_arc(cid)?;
    collect_added_range_from_node(prolly, &node, span_end, range_start, range_end, diffs)
}

fn collect_added_range_from_node<S: Store>(
    prolly: &Prolly<S>,
    node: &Node,
    span_end: Option<&[u8]>,
    range_start: &[u8],
    range_end: Option<&[u8]>,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    if node.leaf {
        let mut idx = lower_bound(&node.keys, range_start);
        while idx < node.len() && key_in_range(&node.keys[idx], range_start, range_end) {
            diffs.push(Diff::Added {
                key: node.keys[idx].clone(),
                val: node.vals[idx].clone(),
            });
            idx += 1;
        }
        return Ok(());
    }

    let child_spans = overlapping_child_cids(node, span_end, range_start, range_end)?;
    let child_cids = child_spans
        .iter()
        .map(|(_, cid)| cid.clone())
        .collect::<Vec<_>>();
    for ((child_end, _), child_node) in child_spans
        .into_iter()
        .zip(prolly.load_many_ordered(&child_cids)?)
    {
        collect_added_range_from_node(
            prolly,
            &child_node,
            child_end,
            range_start,
            range_end,
            diffs,
        )?;
    }

    Ok(())
}

fn collect_removed_range_from_cid<S: Store>(
    prolly: &Prolly<S>,
    cid: &Cid,
    span_end: Option<&[u8]>,
    range_start: &[u8],
    range_end: Option<&[u8]>,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    let node = prolly.load_arc(cid)?;
    collect_removed_range_from_node(prolly, &node, span_end, range_start, range_end, diffs)
}

fn collect_removed_range_from_node<S: Store>(
    prolly: &Prolly<S>,
    node: &Node,
    span_end: Option<&[u8]>,
    range_start: &[u8],
    range_end: Option<&[u8]>,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    if node.leaf {
        let mut idx = lower_bound(&node.keys, range_start);
        while idx < node.len() && key_in_range(&node.keys[idx], range_start, range_end) {
            diffs.push(Diff::Removed {
                key: node.keys[idx].clone(),
                val: node.vals[idx].clone(),
            });
            idx += 1;
        }
        return Ok(());
    }

    let child_spans = overlapping_child_cids(node, span_end, range_start, range_end)?;
    let child_cids = child_spans
        .iter()
        .map(|(_, cid)| cid.clone())
        .collect::<Vec<_>>();
    for ((child_end, _), child_node) in child_spans
        .into_iter()
        .zip(prolly.load_many_ordered(&child_cids)?)
    {
        collect_removed_range_from_node(
            prolly,
            &child_node,
            child_end,
            range_start,
            range_end,
            diffs,
        )?;
    }

    Ok(())
}

fn child_cids(node: &Node) -> Result<Vec<Cid>, Error> {
    (0..node.len()).map(|idx| child_cid(node, idx)).collect()
}

fn overlapping_child_cids<'a>(
    node: &'a Node,
    span_end: Option<&'a [u8]>,
    range_start: &[u8],
    range_end: Option<&[u8]>,
) -> Result<Vec<ChildSpanCid<'a>>, Error> {
    let mut children = Vec::new();
    for idx in 0..node.len() {
        let child_start = node.keys[idx].as_slice();
        let child_end = child_span_end(node, idx, span_end);
        if span_overlaps_range(child_start, child_end, range_start, range_end) {
            children.push((child_end, child_cid(node, idx)?));
        } else if range_ends_before_or_at(range_end, child_start) {
            break;
        }
    }
    Ok(children)
}

fn child_cid(node: &Node, idx: usize) -> Result<Cid, Error> {
    Ok(Cid(node.vals[idx]
        .as_slice()
        .try_into()
        .map_err(|_| Error::InvalidNode)?))
}

fn child_span_end<'a>(node: &'a Node, idx: usize, span_end: Option<&'a [u8]>) -> Option<&'a [u8]> {
    node.keys.get(idx + 1).map(Vec::as_slice).or(span_end)
}

fn span_ends_before_or_at(end: Option<&[u8]>, start: &[u8]) -> bool {
    end.is_some_and(|end| end <= start)
}

fn range_ends_before_or_at(end: Option<&[u8]>, start: &[u8]) -> bool {
    end.is_some_and(|end| end <= start)
}

fn span_overlaps_range(
    span_start: &[u8],
    span_end: Option<&[u8]>,
    range_start: &[u8],
    range_end: Option<&[u8]>,
) -> bool {
    !span_ends_before_or_at(span_end, range_start)
        && !range_ends_before_or_at(range_end, span_start)
}

fn key_in_range(key: &[u8], start: &[u8], end: Option<&[u8]>) -> bool {
    key >= start
        && match end {
            Some(end) => key < end,
            None => true,
        }
}

fn lower_bound(keys: &[Vec<u8>], key: &[u8]) -> usize {
    keys.partition_point(|candidate| candidate.as_slice() < key)
}

/// Merge two trees using three-way merge.
///
/// Performs a three-way merge using `base` as the common ancestor.
/// Changes from both `left` and `right` are combined into a single tree.
/// Uses the diff algorithm to efficiently identify entries to add.
///
/// # Arguments
/// * `prolly` - The Prolly tree manager
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
pub fn merge_trees<S: Store>(
    prolly: &Prolly<S>,
    base: &Tree,
    left: &Tree,
    right: &Tree,
    resolver: Option<Resolver>,
) -> Result<Tree, Error> {
    if left.root == right.root {
        return Ok(left.clone());
    }
    if left.root == base.root {
        return Ok(right.clone());
    }
    if right.root == base.root {
        return Ok(left.clone());
    }

    if let Some(merged) = try_structural_merge(prolly, base, left, right, resolver.as_deref())? {
        return Ok(merged);
    }

    let right_diff = compute_diff(prolly, base, right)?;
    merge_trees_with_right_diff(prolly, base, left, &right_diff, resolver)
}

fn try_structural_merge<S: Store>(
    prolly: &Prolly<S>,
    base: &Tree,
    left: &Tree,
    right: &Tree,
    resolver: Option<&dyn Fn(&Conflict) -> Option<Vec<u8>>>,
) -> Result<Option<Tree>, Error> {
    let (Some(base_cid), Some(left_cid), Some(right_cid)) = (&base.root, &left.root, &right.root)
    else {
        return Ok(None);
    };

    let mut collector = BatchWriteCollector::new();
    let Some(root) = try_structural_merge_cids(
        prolly,
        base_cid,
        left_cid,
        right_cid,
        resolver,
        &mut collector,
    )?
    else {
        return Ok(None);
    };
    collector.flush(prolly.store())?;

    Ok(Some(Tree {
        root: Some(root),
        config: base.config.clone(),
    }))
}

fn try_structural_merge_cids<S: Store>(
    prolly: &Prolly<S>,
    base_cid: &Cid,
    left_cid: &Cid,
    right_cid: &Cid,
    resolver: Option<&dyn Fn(&Conflict) -> Option<Vec<u8>>>,
    collector: &mut BatchWriteCollector,
) -> Result<Option<Cid>, Error> {
    if left_cid == right_cid {
        return Ok(Some(left_cid.clone()));
    }
    if left_cid == base_cid {
        return Ok(Some(right_cid.clone()));
    }
    if right_cid == base_cid {
        return Ok(Some(left_cid.clone()));
    }

    let nodes =
        prolly.load_many_ordered(&[base_cid.clone(), left_cid.clone(), right_cid.clone()])?;
    let base = nodes[0].clone();
    let left = nodes[1].clone();
    let right = nodes[2].clone();

    if base.leaf != left.leaf
        || base.leaf != right.leaf
        || base.level != left.level
        || base.level != right.level
        || base.keys != left.keys
        || base.keys != right.keys
    {
        return Ok(None);
    }

    if base.leaf {
        return try_structural_merge_leaf(
            prolly, &base, &left, &right, base_cid, left_cid, right_cid, resolver, collector,
        )
        .map(Some);
    }

    try_structural_merge_internal(
        prolly, &base, &left, &right, base_cid, left_cid, right_cid, resolver, collector,
    )
}

fn try_structural_merge_internal<S: Store>(
    prolly: &Prolly<S>,
    base: &Node,
    left: &Node,
    right: &Node,
    base_cid: &Cid,
    left_cid: &Cid,
    right_cid: &Cid,
    resolver: Option<&dyn Fn(&Conflict) -> Option<Vec<u8>>>,
    collector: &mut BatchWriteCollector,
) -> Result<Option<Cid>, Error> {
    if base.len() != left.len() || base.len() != right.len() {
        return Ok(None);
    }

    let mut merged = prolly.new_node_like(base);
    let mut differs_from_base = false;
    prefetch_structural_merge_frontier(prolly, base, left, right);

    for idx in 0..base.len() {
        let base_child = child_cid(base, idx)?;
        let left_child = child_cid(left, idx)?;
        let right_child = child_cid(right, idx)?;
        let Some(merged_child) = try_structural_merge_cids(
            prolly,
            &base_child,
            &left_child,
            &right_child,
            resolver,
            collector,
        )?
        else {
            return Ok(None);
        };

        if merged_child != base_child {
            differs_from_base = true;
        }
        merged.keys.push(base.keys[idx].clone());
        merged.vals.push(merged_child.0.to_vec());
    }

    if !differs_from_base {
        return Ok(Some(base_cid.clone()));
    }
    if merged.vals == left.vals {
        return Ok(Some(left_cid.clone()));
    }
    if merged.vals == right.vals {
        return Ok(Some(right_cid.clone()));
    }

    Ok(Some(collector.add(&merged)))
}

fn prefetch_structural_merge_frontier<S: Store>(
    prolly: &Prolly<S>,
    base: &Node,
    left: &Node,
    right: &Node,
) {
    if !prolly.store().prefers_batch_reads() || base.len() <= 1 {
        return;
    }

    let mut cids = Vec::new();
    for idx in 0..base.len() {
        let (Ok(base_child), Ok(left_child), Ok(right_child)) = (
            child_cid(base, idx),
            child_cid(left, idx),
            child_cid(right, idx),
        ) else {
            continue;
        };

        if left_child == right_child || left_child == base_child || right_child == base_child {
            continue;
        }

        cids.push(base_child);
        cids.push(left_child);
        cids.push(right_child);
    }

    if cids.len() > 3 {
        let _ = prolly.load_many_ordered(&cids);
    }
}

#[allow(clippy::too_many_arguments)]
fn try_structural_merge_leaf<S: Store>(
    prolly: &Prolly<S>,
    base: &Node,
    left: &Node,
    right: &Node,
    base_cid: &Cid,
    left_cid: &Cid,
    right_cid: &Cid,
    resolver: Option<&dyn Fn(&Conflict) -> Option<Vec<u8>>>,
    collector: &mut BatchWriteCollector,
) -> Result<Cid, Error> {
    let mut merged = prolly.new_node_like(base);
    merged.keys = base.keys.clone();

    for idx in 0..base.len() {
        let base_val = &base.vals[idx];
        let left_val = &left.vals[idx];
        let right_val = &right.vals[idx];
        let merged_val = if left_val == right_val {
            left_val.clone()
        } else if left_val == base_val {
            right_val.clone()
        } else if right_val == base_val {
            left_val.clone()
        } else {
            let conflict = Conflict {
                key: base.keys[idx].clone(),
                base: Some(base_val.clone()),
                left: left_val.clone(),
                right: right_val.clone(),
            };
            if let Some(resolve) = resolver {
                resolve(&conflict).ok_or(Error::Conflict(conflict))?
            } else {
                return Err(Error::Conflict(conflict));
            }
        };
        merged.vals.push(merged_val);
    }

    if merged.vals == base.vals {
        return Ok(base_cid.clone());
    }
    if merged.vals == left.vals {
        return Ok(left_cid.clone());
    }
    if merged.vals == right.vals {
        return Ok(right_cid.clone());
    }

    Ok(collector.add(&merged))
}

pub(crate) fn try_append_only_diff<S: Store>(
    prolly: &Prolly<S>,
    base: &Tree,
    other: &Tree,
) -> Result<Option<Vec<Diff>>, Error> {
    if base.root == other.root {
        return Ok(Some(Vec::new()));
    }

    let mut diffs = Vec::new();
    match (&base.root, &other.root) {
        (None, Some(other_cid)) => {
            collect_added_from_cid(prolly, other_cid, &mut diffs)?;
            Ok(Some(diffs))
        }
        (Some(_), None) => Ok(None),
        (Some(base_cid), Some(other_cid)) => {
            let nodes = prolly.load_many_ordered(&[base_cid.clone(), other_cid.clone()])?;
            let base_node = nodes[0].clone();
            let other_node = nodes[1].clone();
            if append_only_diff_nodes(prolly, &base_node, &other_node, &mut diffs)? {
                Ok(Some(diffs))
            } else {
                Ok(None)
            }
        }
        (None, None) => Ok(Some(Vec::new())),
    }
}

fn append_only_diff_nodes<S: Store>(
    prolly: &Prolly<S>,
    base: &Node,
    other: &Node,
    diffs: &mut Vec<Diff>,
) -> Result<bool, Error> {
    if other.level > base.level {
        if other.leaf || other.is_empty() {
            return Ok(false);
        }

        let first_child = child_cid(other, 0)?;
        let first_child_node = prolly.load_arc(&first_child)?;
        if !append_only_diff_nodes(prolly, base, &first_child_node, diffs)? {
            return Ok(false);
        }

        for idx in 1..other.len() {
            let child = child_cid(other, idx)?;
            collect_added_from_cid(prolly, &child, diffs)?;
        }

        return Ok(true);
    }

    if base.level != other.level || base.leaf != other.leaf || other.len() < base.len() {
        return Ok(false);
    }

    if base.leaf {
        for idx in 0..base.len() {
            if base.keys[idx] != other.keys[idx] || base.vals[idx] != other.vals[idx] {
                return Ok(false);
            }
        }

        for idx in base.len()..other.len() {
            diffs.push(Diff::Added {
                key: other.keys[idx].clone(),
                val: other.vals[idx].clone(),
            });
        }

        return Ok(true);
    }

    if base.is_empty() {
        for idx in 0..other.len() {
            let child = child_cid(other, idx)?;
            collect_added_from_cid(prolly, &child, diffs)?;
        }
        return Ok(true);
    }

    let right_edge_idx = base.len() - 1;
    for idx in 0..right_edge_idx {
        if base.keys[idx] != other.keys[idx] || base.vals[idx] != other.vals[idx] {
            return Ok(false);
        }
    }

    if base.keys[right_edge_idx] != other.keys[right_edge_idx] {
        return Ok(false);
    }

    let base_child = child_cid(base, right_edge_idx)?;
    let other_child = child_cid(other, right_edge_idx)?;
    if base_child != other_child {
        let nodes = prolly.load_many_ordered(&[base_child, other_child])?;
        let base_child_node = nodes[0].clone();
        let other_child_node = nodes[1].clone();
        if !append_only_diff_nodes(prolly, &base_child_node, &other_child_node, diffs)? {
            return Ok(false);
        }
    } else {
        prolly.load_arc(&base_child)?;
    }

    for idx in base.len()..other.len() {
        let child = child_cid(other, idx)?;
        collect_added_from_cid(prolly, &child, diffs)?;
    }

    Ok(true)
}

fn merge_trees_with_right_diff<S: Store>(
    prolly: &Prolly<S>,
    base: &Tree,
    left: &Tree,
    right_diff: &[Diff],
    resolver: Option<Resolver>,
) -> Result<Tree, Error> {
    let right_changes = build_merge_change_map(right_diff);
    if right_changes_are_append_only_after(prolly, base, left, &right_changes)? {
        let mutations = right_changes
            .iter()
            .filter_map(|(key, change)| {
                change.value.as_ref().map(|val| Mutation::Upsert {
                    key: key.clone(),
                    val: val.clone(),
                })
            })
            .collect::<Vec<_>>();
        return prolly.batch(left, mutations);
    }

    let mut mutations = Vec::new();
    let keys = right_changes.keys().cloned().collect::<Vec<_>>();
    let left_values = get_many_from_tree(prolly, left, &keys)?;

    for ((key, change), left_val) in right_changes.iter().zip(left_values) {
        let base_val = &change.base;
        let right_val = &change.value;

        if &left_val == base_val {
            push_change_mutation(&mut mutations, key, right_val);
            continue;
        }

        if option_bytes_eq(&left_val, right_val) {
            continue;
        }

        let conflict =
            build_conflict_from_values(key, base_val.clone(), left_val, right_val.clone());
        if let Some(ref resolve) = resolver {
            if let Some(resolved) = resolve(&conflict) {
                mutations.push(Mutation::Upsert {
                    key: key.clone(),
                    val: resolved,
                });
                continue;
            }
        }

        return Err(Error::Conflict(conflict));
    }

    if mutations.is_empty() {
        Ok(left.clone())
    } else {
        prolly.batch(left, mutations)
    }
}

fn push_change_mutation(mutations: &mut Vec<Mutation>, key: &[u8], value: &Option<Vec<u8>>) {
    match value {
        Some(val) => mutations.push(Mutation::Upsert {
            key: key.to_vec(),
            val: val.clone(),
        }),
        None => mutations.push(Mutation::Delete { key: key.to_vec() }),
    }
}

fn option_bytes_eq(left: &Option<Vec<u8>>, right: &Option<Vec<u8>>) -> bool {
    left.as_deref() == right.as_deref()
}

struct KeyLookupFrame {
    cid: Cid,
    positions: Vec<usize>,
}

fn get_many_from_tree<S: Store>(
    prolly: &Prolly<S>,
    tree: &Tree,
    keys: &[Vec<u8>],
) -> Result<Vec<Option<Vec<u8>>>, Error> {
    let mut values = vec![None; keys.len()];
    let Some(root_cid) = &tree.root else {
        return Ok(values);
    };

    if keys.is_empty() {
        return Ok(values);
    }

    let mut frames = vec![KeyLookupFrame {
        cid: root_cid.clone(),
        positions: (0..keys.len()).collect(),
    }];

    while !frames.is_empty() {
        let cids = frames
            .iter()
            .map(|frame| frame.cid.clone())
            .collect::<Vec<_>>();
        let nodes = prolly.load_many_ordered(&cids)?;
        let mut next_frames = Vec::new();

        for (frame, node) in frames.into_iter().zip(nodes) {
            if node.leaf {
                for position in frame.positions {
                    if let Ok(idx) = node.search(&keys[position]) {
                        values[position] = Some(node.vals[idx].clone());
                    }
                }
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

fn route_key_positions_to_children(
    node: &Node,
    positions: Vec<usize>,
    keys: &[Vec<u8>],
) -> Result<Vec<KeyLookupFrame>, Error> {
    if node.is_empty() {
        return Err(Error::InvalidNode);
    }

    let mut routed: Vec<(usize, Vec<usize>)> = Vec::new();
    let mut child_index = 0usize;

    for position in positions {
        let key = keys[position].as_slice();
        while child_index + 1 < node.len() && key >= node.keys[child_index + 1].as_slice() {
            child_index += 1;
        }

        match routed.last_mut() {
            Some((idx, bucket)) if *idx == child_index => bucket.push(position),
            _ => routed.push((child_index, vec![position])),
        }
    }

    routed
        .into_iter()
        .map(|(idx, positions)| {
            Ok(KeyLookupFrame {
                cid: child_cid(node, idx)?,
                positions,
            })
        })
        .collect()
}

fn right_changes_are_append_only_after<S: Store>(
    prolly: &Prolly<S>,
    base: &Tree,
    left: &Tree,
    right_changes: &BTreeMap<Vec<u8>, MergeChange>,
) -> Result<bool, Error> {
    let Some((first_key, _)) = right_changes.first_key_value() else {
        return Ok(false);
    };

    if right_changes.values().any(|change| change.value.is_none()) {
        return Ok(false);
    }

    Ok(key_is_after_tree(prolly, first_key, base)? && key_is_after_tree(prolly, first_key, left)?)
}

fn key_is_after_tree<S: Store>(prolly: &Prolly<S>, key: &[u8], tree: &Tree) -> Result<bool, Error> {
    let Some(root_cid) = &tree.root else {
        return Ok(true);
    };

    Ok(get_max_key(prolly, root_cid)?
        .as_deref()
        .map_or(true, |max_key| key > max_key))
}

/// Build a change map from a list of diffs.
///
/// Maps each key to its new value (Some for additions/changes, None for deletions).
pub fn build_change_map(diffs: &[Diff]) -> BTreeMap<Vec<u8>, Option<Vec<u8>>> {
    diffs
        .iter()
        .map(|d| match d {
            Diff::Added { key, val } | Diff::Changed { key, new: val, .. } => {
                (key.clone(), Some(val.clone()))
            }
            Diff::Removed { key, .. } => (key.clone(), None),
        })
        .collect()
}

fn build_merge_change_map(diffs: &[Diff]) -> BTreeMap<Vec<u8>, MergeChange> {
    diffs
        .iter()
        .map(|d| match d {
            Diff::Added { key, val } => (
                key.clone(),
                MergeChange {
                    base: None,
                    value: Some(val.clone()),
                },
            ),
            Diff::Removed { key, val } => (
                key.clone(),
                MergeChange {
                    base: Some(val.clone()),
                    value: None,
                },
            ),
            Diff::Changed { key, old, new } => (
                key.clone(),
                MergeChange {
                    base: Some(old.clone()),
                    value: Some(new.clone()),
                },
            ),
        })
        .collect()
}

fn build_conflict_from_values(
    key: &[u8],
    base: Option<Vec<u8>>,
    left: Option<Vec<u8>>,
    right: Option<Vec<u8>>,
) -> Conflict {
    Conflict {
        key: key.to_vec(),
        base,
        left: left.unwrap_or_default(),
        right: right.unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::builder::BatchBuilder;
    use super::super::config::Config;
    use super::super::store::{BatchOp, MemStore, Store};
    use super::*;
    use std::collections::{BTreeMap, HashSet};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

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
        blocked_get_keys: Mutex<HashSet<Vec<u8>>>,
        prefer_batch_reads: bool,
        get_calls: AtomicUsize,
        batch_get_ordered_calls: AtomicUsize,
        max_batch_get_ordered_len: AtomicUsize,
    }

    impl CountingStore {
        fn reset_counts(&self) {
            self.get_calls.store(0, Ordering::Relaxed);
            self.batch_get_ordered_calls.store(0, Ordering::Relaxed);
            self.max_batch_get_ordered_len.store(0, Ordering::Relaxed);
        }

        fn block_get_key(&self, key: &[u8]) {
            self.blocked_get_keys.lock().unwrap().insert(key.to_vec());
        }

        fn clear_blocked_get_keys(&self) {
            self.blocked_get_keys.lock().unwrap().clear();
        }

        fn ensure_get_allowed(&self, key: &[u8]) -> Result<(), CountingStoreError> {
            if self.blocked_get_keys.lock().unwrap().contains(key) {
                return Err(CountingStoreError);
            }
            Ok(())
        }
    }

    impl Store for CountingStore {
        type Error = CountingStoreError;

        fn get(&self, key: &[u8]) -> Result<Option<Vec<u8>>, Self::Error> {
            self.ensure_get_allowed(key)?;
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

        fn batch_get_ordered(&self, keys: &[&[u8]]) -> Result<Vec<Option<Vec<u8>>>, Self::Error> {
            for key in keys {
                self.ensure_get_allowed(key)?;
            }
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
    fn diff_collectors_batch_hydrate_added_subtrees() {
        let store = Arc::new(CountingStore::default());
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(4)
            .chunking_factor(u32::MAX)
            .build();
        let mut builder = BatchBuilder::new(store.clone(), config.clone());
        for i in 0..96 {
            builder.add(
                format!("k{i:03}").into_bytes(),
                format!("v{i:03}").into_bytes(),
            );
        }
        let tree = builder.build().unwrap();
        let empty = Tree {
            root: None,
            config: config.clone(),
        };

        let prolly = Prolly::new(store.clone(), config);
        store.reset_counts();
        let diffs = compute_diff(&prolly, &empty, &tree).unwrap();

        assert_eq!(diffs.len(), 96);
        assert!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed) > 0,
            "added subtree collection should hydrate internal children in ordered batches"
        );
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            1,
            "only the added root should require a single-key get"
        );
    }

    #[test]
    fn structural_stream_diff_skips_unchanged_subtrees() {
        let store = Arc::new(CountingStore::default());
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(4)
            .chunking_factor(u32::MAX)
            .build();
        let mut builder = BatchBuilder::new(store.clone(), config.clone());
        for i in 0..256 {
            builder.add(
                format!("k{i:03}").into_bytes(),
                format!("v{i:03}").into_bytes(),
            );
        }
        let base = builder.build().unwrap();
        let prolly = Prolly::new(store.clone(), config);
        let other = prolly
            .put(&base, b"k173".to_vec(), b"changed-173".to_vec())
            .unwrap();

        prolly.clear_cache();
        store.reset_counts();
        let diffs = stream_diff(&prolly, &base, &other)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(
            diffs,
            vec![Diff::Changed {
                key: b"k173".to_vec(),
                old: b"v173".to_vec(),
                new: b"changed-173".to_vec(),
            }]
        );
        assert!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed) > 0,
            "structural streaming should hydrate compared node pairs in ordered batches"
        );
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            0,
            "structural streaming should not cursor-scan entries with single-key gets"
        );
    }

    #[test]
    fn structural_stream_diff_prefetches_sibling_frames_for_batched_stores() {
        let store = Arc::new(CountingStore {
            prefer_batch_reads: true,
            ..CountingStore::default()
        });
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(4)
            .chunking_factor(u32::MAX)
            .build();
        let mut builder = BatchBuilder::new(store.clone(), config.clone());
        for i in 0..512 {
            builder.add(
                format!("k{i:03}").into_bytes(),
                format!("v{i:03}").into_bytes(),
            );
        }
        let base = builder.build().unwrap();
        let prolly = Prolly::new(store.clone(), config);
        let other = prolly
            .batch(
                &base,
                (0..512)
                    .step_by(29)
                    .map(|i| Mutation::Upsert {
                        key: format!("k{i:03}").into_bytes(),
                        val: format!("changed-{i:03}").into_bytes(),
                    })
                    .collect(),
            )
            .unwrap();

        prolly.clear_cache();
        store.reset_counts();
        let diffs = stream_diff(&prolly, &base, &other)
            .collect::<Result<Vec<_>, _>>()
            .unwrap();

        assert_eq!(diffs.len(), 18);
        assert!(
            store.max_batch_get_ordered_len.load(Ordering::Relaxed) > 2,
            "batched-read stores should prefetch sibling diff frames wider than one node pair"
        );
    }

    #[test]
    fn eager_diff_uses_structural_prefetch_for_batched_stores() {
        let store = Arc::new(CountingStore {
            prefer_batch_reads: true,
            ..CountingStore::default()
        });
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(4)
            .chunking_factor(u32::MAX)
            .build();
        let mut builder = BatchBuilder::new(store.clone(), config.clone());
        for i in 0..512 {
            builder.add(
                format!("k{i:03}").into_bytes(),
                format!("v{i:03}").into_bytes(),
            );
        }
        let base = builder.build().unwrap();
        let prolly = Prolly::new(store.clone(), config);
        let other = prolly
            .batch(
                &base,
                (0..512)
                    .step_by(29)
                    .map(|i| Mutation::Upsert {
                        key: format!("k{i:03}").into_bytes(),
                        val: format!("changed-{i:03}").into_bytes(),
                    })
                    .collect(),
            )
            .unwrap();

        prolly.clear_cache();
        store.reset_counts();
        let diffs = compute_diff(&prolly, &base, &other).unwrap();

        assert_eq!(diffs.len(), 18);
        assert!(
            store.max_batch_get_ordered_len.load(Ordering::Relaxed) > 2,
            "eager diff should reuse structural sibling prefetch for batched-read stores"
        );
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            0,
            "eager structural diff should avoid point reads after append-only fallback"
        );
    }

    #[test]
    fn test_compute_diff_same_tree() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let tree = prolly.create();

        let tree = prolly.put(&tree, b"a".to_vec(), b"1".to_vec()).unwrap();

        // Diff of same tree should be empty
        let diffs = compute_diff(&prolly, &tree, &tree).unwrap();
        assert!(diffs.is_empty());
    }

    #[test]
    fn test_compute_diff_added_entries() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let other = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();

        let diffs = compute_diff(&prolly, &base, &other).unwrap();

        assert_eq!(diffs.len(), 1);
        assert!(matches!(
            &diffs[0],
            Diff::Added { key, val } if key == b"b" && val == b"2"
        ));
    }

    #[test]
    fn test_compute_diff_removed_entries() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let base = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
        let other = prolly.delete(&base, b"b").unwrap();

        let diffs = compute_diff(&prolly, &base, &other).unwrap();

        assert_eq!(diffs.len(), 1);
        assert!(matches!(
            &diffs[0],
            Diff::Removed { key, val } if key == b"b" && val == b"2"
        ));
    }

    #[test]
    fn test_compute_diff_changed_entries() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let other = prolly.put(&base, b"a".to_vec(), b"2".to_vec()).unwrap();

        let diffs = compute_diff(&prolly, &base, &other).unwrap();

        assert_eq!(diffs.len(), 1);
        assert!(matches!(
            &diffs[0],
            Diff::Changed { key, old, new } if key == b"a" && old == b"1" && new == b"2"
        ));
    }

    #[test]
    fn append_only_diff_detects_right_suffix_additions() {
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(4)
            .chunking_factor(u32::MAX)
            .hash_seed(31)
            .build();
        let prolly = Prolly::new(MemStore::new(), config);
        let mut base = prolly.create();

        for i in 0..32 {
            base = prolly
                .put(
                    &base,
                    format!("k{i:03}").into_bytes(),
                    format!("v{i:03}").into_bytes(),
                )
                .unwrap();
        }

        let mut other = base.clone();
        for i in 32..48 {
            other = prolly
                .put(
                    &other,
                    format!("k{i:03}").into_bytes(),
                    format!("v{i:03}").into_bytes(),
                )
                .unwrap();
        }

        let diffs = try_append_only_diff(&prolly, &base, &other)
            .unwrap()
            .unwrap();
        assert_eq!(diffs.len(), 16);
        assert!(diffs.iter().all(|diff| matches!(diff, Diff::Added { .. })));
        assert!(matches!(
            &diffs[0],
            Diff::Added { key, val } if key == b"k032" && val == b"v032"
        ));

        assert_eq!(compute_diff(&prolly, &base, &other).unwrap(), diffs);
    }

    #[test]
    fn append_only_diff_batches_changed_right_edge_child_pair() {
        let store = Arc::new(CountingStore::default());
        let config = Config::default();
        let prolly = Prolly::new(store.clone(), config.clone());

        let mut leaf_a = prolly.new_leaf_node();
        leaf_a.keys.push(b"a".to_vec());
        leaf_a.vals.push(b"1".to_vec());
        let leaf_a_cid = prolly.save(&leaf_a).unwrap();

        let mut leaf_b = prolly.new_leaf_node();
        leaf_b.keys.push(b"b".to_vec());
        leaf_b.vals.push(b"2".to_vec());
        let leaf_b_cid = prolly.save(&leaf_b).unwrap();

        let mut leaf_bc = prolly.new_leaf_node();
        leaf_bc.keys = vec![b"b".to_vec(), b"c".to_vec()];
        leaf_bc.vals = vec![b"2".to_vec(), b"3".to_vec()];
        let leaf_bc_cid = prolly.save(&leaf_bc).unwrap();

        let mut base_root = prolly.new_internal_node(1);
        base_root.keys = vec![b"a".to_vec(), b"b".to_vec()];
        base_root.vals = vec![leaf_a_cid.0.to_vec(), leaf_b_cid.0.to_vec()];
        let base_root_cid = prolly.save(&base_root).unwrap();

        let mut other_root = prolly.new_internal_node(1);
        other_root.keys = vec![b"a".to_vec(), b"b".to_vec()];
        other_root.vals = vec![leaf_a_cid.0.to_vec(), leaf_bc_cid.0.to_vec()];
        let other_root_cid = prolly.save(&other_root).unwrap();

        let base = Tree {
            root: Some(base_root_cid),
            config: config.clone(),
        };
        let other = Tree {
            root: Some(other_root_cid),
            config,
        };

        prolly.clear_cache();
        store.reset_counts();
        let diffs = try_append_only_diff(&prolly, &base, &other)
            .unwrap()
            .unwrap();

        assert_eq!(
            diffs,
            vec![Diff::Added {
                key: b"c".to_vec(),
                val: b"3".to_vec()
            }]
        );
        assert_eq!(
            store.get_calls.load(Ordering::Relaxed),
            0,
            "changed right-edge child nodes should be loaded through ordered batches"
        );
        assert_eq!(store.batch_get_ordered_calls.load(Ordering::Relaxed), 2);
        assert_eq!(store.max_batch_get_ordered_len.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn append_only_diff_rejects_existing_key_updates() {
        let config = Config::builder()
            .min_chunk_size(2)
            .max_chunk_size(4)
            .chunking_factor(u32::MAX)
            .hash_seed(37)
            .build();
        let prolly = Prolly::new(MemStore::new(), config);
        let mut base = prolly.create();

        for i in 0..32 {
            base = prolly
                .put(
                    &base,
                    format!("k{i:03}").into_bytes(),
                    format!("v{i:03}").into_bytes(),
                )
                .unwrap();
        }

        let other = prolly
            .put(&base, b"k010".to_vec(), b"updated".to_vec())
            .unwrap();

        assert!(try_append_only_diff(&prolly, &base, &other)
            .unwrap()
            .is_none());

        assert!(matches!(
            &compute_diff(&prolly, &base, &other).unwrap()[0],
            Diff::Changed { key, old, new }
                if key == b"k010" && old == b"v010" && new == b"updated"
        ));
    }

    #[test]
    fn test_merge_trees_no_conflicts() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let left = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();
        let right = prolly.put(&base, b"c".to_vec(), b"3".to_vec()).unwrap();

        let merged = merge_trees(&prolly, &base, &left, &right, None).unwrap();

        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"1".to_vec()));
        assert_eq!(prolly.get(&merged, b"b").unwrap(), Some(b"2".to_vec()));
        assert_eq!(prolly.get(&merged, b"c").unwrap(), Some(b"3".to_vec()));
    }

    #[test]
    fn merge_returns_changed_branch_without_reads_when_other_branch_unchanged() {
        let store = Arc::new(CountingStore::default());
        let prolly = Prolly::new(store.clone(), Config::default());
        let base = prolly
            .put(&prolly.create(), b"a".to_vec(), b"1".to_vec())
            .unwrap();
        let right = prolly.put(&base, b"b".to_vec(), b"2".to_vec()).unwrap();

        prolly.clear_cache();
        store.reset_counts();
        let merged = merge_trees(&prolly, &base, &base, &right, None).unwrap();

        assert_eq!(merged.root, right.root);
        assert_eq!(store.get_calls.load(Ordering::Relaxed), 0);
        assert_eq!(store.batch_get_ordered_calls.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn structural_merge_reuses_disjoint_changed_subtrees_without_reading_them() {
        let store = Arc::new(CountingStore {
            prefer_batch_reads: true,
            ..CountingStore::default()
        });
        let prolly = Prolly::new(store.clone(), Config::default());

        let mut base_left_leaf = prolly.new_leaf_node();
        base_left_leaf.keys.push(b"a".to_vec());
        base_left_leaf.vals.push(b"1".to_vec());
        let base_left_leaf_cid = prolly.save(&base_left_leaf).unwrap();

        let mut base_right_leaf = prolly.new_leaf_node();
        base_right_leaf.keys.push(b"m".to_vec());
        base_right_leaf.vals.push(b"1".to_vec());
        let base_right_leaf_cid = prolly.save(&base_right_leaf).unwrap();

        let mut base_root = prolly.new_internal_node(1);
        base_root.keys = vec![b"a".to_vec(), b"m".to_vec()];
        base_root.vals = vec![
            base_left_leaf_cid.0.to_vec(),
            base_right_leaf_cid.0.to_vec(),
        ];
        let base_root_cid = prolly.save(&base_root).unwrap();

        let mut left_leaf = prolly.new_leaf_node();
        left_leaf.keys.push(b"a".to_vec());
        left_leaf.vals.push(b"left".to_vec());
        let left_leaf_cid = prolly.save(&left_leaf).unwrap();

        let mut left_root = prolly.new_internal_node(1);
        left_root.keys = base_root.keys.clone();
        left_root.vals = vec![left_leaf_cid.0.to_vec(), base_right_leaf_cid.0.to_vec()];
        let left_root_cid = prolly.save(&left_root).unwrap();

        let mut right_leaf = prolly.new_leaf_node();
        right_leaf.keys.push(b"m".to_vec());
        right_leaf.vals.push(b"right".to_vec());
        let right_leaf_cid = prolly.save(&right_leaf).unwrap();

        let mut right_root = prolly.new_internal_node(1);
        right_root.keys = base_root.keys.clone();
        right_root.vals = vec![base_left_leaf_cid.0.to_vec(), right_leaf_cid.0.to_vec()];
        let right_root_cid = prolly.save(&right_root).unwrap();

        let base = Tree {
            root: Some(base_root_cid),
            config: Config::default(),
        };
        let left = Tree {
            root: Some(left_root_cid),
            config: Config::default(),
        };
        let right = Tree {
            root: Some(right_root_cid),
            config: Config::default(),
        };

        store.block_get_key(left_leaf_cid.as_bytes());
        store.block_get_key(right_leaf_cid.as_bytes());
        prolly.clear_cache();
        store.reset_counts();

        let merged = merge_trees(&prolly, &base, &left, &right, None).unwrap();

        assert!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed) > 0,
            "structural merge should batch-read only the internal merge frontier"
        );

        store.clear_blocked_get_keys();
        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"left".to_vec()));
        assert_eq!(prolly.get(&merged, b"m").unwrap(), Some(b"right".to_vec()));
    }

    #[test]
    fn structural_merge_prefetches_sibling_merge_frontier() {
        let store = Arc::new(CountingStore {
            prefer_batch_reads: true,
            ..CountingStore::default()
        });
        let prolly = Prolly::new(store.clone(), Config::default());

        let save_leaf = |key: &[u8], val: &[u8]| {
            let mut leaf = prolly.new_leaf_node();
            leaf.keys.push(key.to_vec());
            leaf.vals.push(val.to_vec());
            prolly.save(&leaf).unwrap()
        };
        let save_internal = |level, keys: Vec<Vec<u8>>, child_cids: Vec<Cid>| {
            let mut node = prolly.new_internal_node(level);
            node.keys = keys;
            node.vals = child_cids
                .into_iter()
                .map(|cid| cid.0.to_vec())
                .collect::<Vec<_>>();
            prolly.save(&node).unwrap()
        };

        let base_a = save_leaf(b"a", b"base-a");
        let base_g = save_leaf(b"g", b"base-g");
        let base_m = save_leaf(b"m", b"base-m");
        let base_t = save_leaf(b"t", b"base-t");
        let base_left_internal = save_internal(
            1,
            vec![b"a".to_vec(), b"g".to_vec()],
            vec![base_a.clone(), base_g.clone()],
        );
        let base_right_internal = save_internal(
            1,
            vec![b"m".to_vec(), b"t".to_vec()],
            vec![base_m.clone(), base_t.clone()],
        );
        let base_root = save_internal(
            2,
            vec![b"a".to_vec(), b"m".to_vec()],
            vec![base_left_internal.clone(), base_right_internal.clone()],
        );

        let left_a = save_leaf(b"a", b"left-a");
        let left_m = save_leaf(b"m", b"left-m");
        let left_left_internal = save_internal(
            1,
            vec![b"a".to_vec(), b"g".to_vec()],
            vec![left_a, base_g.clone()],
        );
        let left_right_internal = save_internal(
            1,
            vec![b"m".to_vec(), b"t".to_vec()],
            vec![left_m, base_t.clone()],
        );
        let left_root = save_internal(
            2,
            vec![b"a".to_vec(), b"m".to_vec()],
            vec![left_left_internal, left_right_internal],
        );

        let right_g = save_leaf(b"g", b"right-g");
        let right_t = save_leaf(b"t", b"right-t");
        let right_left_internal =
            save_internal(1, vec![b"a".to_vec(), b"g".to_vec()], vec![base_a, right_g]);
        let right_right_internal =
            save_internal(1, vec![b"m".to_vec(), b"t".to_vec()], vec![base_m, right_t]);
        let right_root = save_internal(
            2,
            vec![b"a".to_vec(), b"m".to_vec()],
            vec![right_left_internal, right_right_internal],
        );

        let base = Tree {
            root: Some(base_root),
            config: Config::default(),
        };
        let left = Tree {
            root: Some(left_root),
            config: Config::default(),
        };
        let right = Tree {
            root: Some(right_root),
            config: Config::default(),
        };

        prolly.clear_cache();
        store.reset_counts();
        let merged = merge_trees(&prolly, &base, &left, &right, None).unwrap();

        assert!(
            store.max_batch_get_ordered_len.load(Ordering::Relaxed) >= 6,
            "structural merge should prefetch sibling child triples wider than one subtree"
        );
        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"left-a".to_vec()));
        assert_eq!(
            prolly.get(&merged, b"g").unwrap(),
            Some(b"right-g".to_vec())
        );
        assert_eq!(prolly.get(&merged, b"m").unwrap(), Some(b"left-m".to_vec()));
        assert_eq!(
            prolly.get(&merged, b"t").unwrap(),
            Some(b"right-t".to_vec())
        );
    }

    #[test]
    fn broad_merge_does_not_read_left_only_changed_subtrees() {
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

        let mut builder = BatchBuilder::new(store.clone(), config.clone());
        for i in 0..4096 {
            builder.add(key_for(i), format!("v{i:04}").into_bytes());
        }
        let base = builder.build().unwrap();
        let prolly = Prolly::new(store.clone(), config);

        let left = prolly
            .batch(
                &base,
                (0..32)
                    .map(|i| Mutation::Upsert {
                        key: key_for(i),
                        val: format!("left-{i:04}").into_bytes(),
                    })
                    .collect(),
            )
            .unwrap();
        let right = prolly
            .batch(
                &base,
                (2000..3100)
                    .map(|i| Mutation::Upsert {
                        key: key_for(i),
                        val: format!("right-{i:04}").into_bytes(),
                    })
                    .collect(),
            )
            .unwrap();

        let left_only_leaf = prolly
            .find_path(&left, &key_for(0))
            .unwrap()
            .last()
            .map(|(node, _)| node.clone())
            .unwrap();
        let blocked_cid = Cid::from_bytes(&left_only_leaf.to_bytes());
        store.block_get_key(blocked_cid.as_bytes());
        prolly.clear_cache();
        store.reset_counts();

        let merged = merge_trees(&prolly, &base, &left, &right, None).unwrap();

        assert!(
            store.batch_get_ordered_calls.load(Ordering::Relaxed) > 0,
            "merge should check left values through batched tree reads"
        );

        store.clear_blocked_get_keys();
        assert_eq!(
            prolly.get(&merged, &key_for(0)).unwrap(),
            Some(b"left-0000".to_vec())
        );
        assert_eq!(
            prolly.get(&merged, &key_for(2500)).unwrap(),
            Some(b"right-2500".to_vec())
        );
    }

    #[test]
    fn test_merge_trees_with_conflict() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let left = prolly.put(&base, b"a".to_vec(), b"left".to_vec()).unwrap();
        let right = prolly.put(&base, b"a".to_vec(), b"right".to_vec()).unwrap();

        let result = merge_trees(&prolly, &base, &left, &right, None);

        assert!(matches!(result, Err(Error::Conflict(_))));
    }

    #[test]
    fn test_merge_trees_with_resolver() {
        let store = MemStore::new();
        let prolly = Prolly::new(store, Config::default());
        let base = prolly.create();

        let base = prolly.put(&base, b"a".to_vec(), b"1".to_vec()).unwrap();
        let left = prolly.put(&base, b"a".to_vec(), b"left".to_vec()).unwrap();
        let right = prolly.put(&base, b"a".to_vec(), b"right".to_vec()).unwrap();

        // Resolver that prefers left
        let resolver: Resolver = Box::new(|c| Some(c.left.clone()));
        let merged = merge_trees(&prolly, &base, &left, &right, Some(resolver)).unwrap();

        assert_eq!(prolly.get(&merged, b"a").unwrap(), Some(b"left".to_vec()));
    }

    #[test]
    fn test_build_change_map() {
        let diffs = vec![
            Diff::Added {
                key: b"a".to_vec(),
                val: b"1".to_vec(),
            },
            Diff::Changed {
                key: b"b".to_vec(),
                old: b"old".to_vec(),
                new: b"new".to_vec(),
            },
            Diff::Removed {
                key: b"c".to_vec(),
                val: b"3".to_vec(),
            },
        ];

        let change_map = build_change_map(&diffs);

        assert_eq!(change_map.get(b"a".as_slice()), Some(&Some(b"1".to_vec())));
        assert_eq!(
            change_map.get(b"b".as_slice()),
            Some(&Some(b"new".to_vec()))
        );
        assert_eq!(change_map.get(b"c".as_slice()), Some(&None));
    }
}
