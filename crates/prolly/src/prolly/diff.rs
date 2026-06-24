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
//! 1. Compute diffs from `base` to `left` and from `base` to `right`
//! 2. Build change maps for both branches
//! 3. Start with the `left` tree as the result
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

use std::collections::BTreeMap;

use super::cid::Cid;
use super::error::{Conflict, Diff, Error, Mutation, Resolver};
use super::node::Node;
use super::store::Store;
use super::tree::Tree;

use super::Prolly;

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
    // Short-circuit: same root CID means no differences
    if base.root == other.root {
        return Ok(vec![]);
    }

    let mut diffs = Vec::new();

    match (&base.root, &other.root) {
        (Some(base_cid), Some(other_cid)) => {
            diff_nodes(prolly, base_cid, other_cid, None, &mut diffs)?;
        }
        (Some(base_cid), None) => {
            collect_removed_from_cid(prolly, base_cid, &mut diffs)?;
        }
        (None, Some(other_cid)) => {
            collect_added_from_cid(prolly, other_cid, &mut diffs)?;
        }
        (None, None) => {}
    }

    Ok(diffs)
}

fn diff_nodes<S: Store>(
    prolly: &Prolly<S>,
    base_cid: &Cid,
    other_cid: &Cid,
    span_end: Option<&[u8]>,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    if base_cid == other_cid {
        return Ok(());
    }

    let base_node = prolly.load_arc(base_cid)?;
    let other_node = prolly.load_arc(other_cid)?;

    match (base_node.leaf, other_node.leaf) {
        (true, true) => diff_leaf_nodes(&base_node, &other_node, diffs),
        (false, false) if base_node.level == other_node.level => {
            diff_internal_nodes(prolly, &base_node, &other_node, span_end, diffs)
        }
        _ => diff_collected_nodes(prolly, &base_node, &other_node, diffs),
    }
}

fn diff_internal_nodes<S: Store>(
    prolly: &Prolly<S>,
    base: &Node,
    other: &Node,
    span_end: Option<&[u8]>,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
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
            diff_nodes(prolly, &base_cid, &other_cid, base_end, diffs)?;
            base_idx += 1;
            other_idx += 1;
        } else if span_ends_before_or_at(base_end, other_start) {
            let base_cid = child_cid(base, base_idx)?;
            collect_removed_from_cid(prolly, &base_cid, diffs)?;
            base_idx += 1;
        } else if span_ends_before_or_at(other_end, base_start) {
            let other_cid = child_cid(other, other_idx)?;
            collect_added_from_cid(prolly, &other_cid, diffs)?;
            other_idx += 1;
        } else {
            // Chunk boundaries overlap but do not line up. Fall back to a local
            // ordered merge for this subtree to preserve correctness.
            return diff_collected_nodes(prolly, base, other, diffs);
        }
    }

    while base_idx < base.len() {
        let base_cid = child_cid(base, base_idx)?;
        collect_removed_from_cid(prolly, &base_cid, diffs)?;
        base_idx += 1;
    }

    while other_idx < other.len() {
        let other_cid = child_cid(other, other_idx)?;
        collect_added_from_cid(prolly, &other_cid, diffs)?;
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

    for child in &node.vals {
        let child_cid = Cid(child
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidNode)?);
        let child_node = prolly.load_arc(&child_cid)?;
        collect_entries_from_node(prolly, &child_node, entries)?;
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

    for child in &node.vals {
        let child_cid = Cid(child
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidNode)?);
        collect_added_from_cid(prolly, &child_cid, diffs)?;
    }

    Ok(())
}

fn collect_removed_from_cid<S: Store>(
    prolly: &Prolly<S>,
    cid: &Cid,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    let node = prolly.load_arc(cid)?;
    collect_removed_from_node(prolly, &node, diffs)
}

fn collect_removed_from_node<S: Store>(
    prolly: &Prolly<S>,
    node: &Node,
    diffs: &mut Vec<Diff>,
) -> Result<(), Error> {
    if node.leaf {
        for (key, val) in node.keys.iter().zip(&node.vals) {
            diffs.push(Diff::Removed {
                key: key.clone(),
                val: val.clone(),
            });
        }
        return Ok(());
    }

    for child in &node.vals {
        let child_cid = Cid(child
            .as_slice()
            .try_into()
            .map_err(|_| Error::InvalidNode)?);
        collect_removed_from_cid(prolly, &child_cid, diffs)?;
    }

    Ok(())
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
    // Get diffs from base to both branches
    let left_diff = compute_diff(prolly, base, left)?;
    let right_diff = compute_diff(prolly, base, right)?;

    // Build change maps: key -> Option<value> (None means deleted)
    let left_changes = build_change_map(&left_diff);
    let right_changes = build_change_map(&right_diff);

    // Start with left tree and batch-apply right-side changes.
    let mut mutations = Vec::new();

    // Apply right changes
    for (key, right_val) in &right_changes {
        let left_val = left_changes.get(key);

        match (left_val, right_val) {
            // Both made the same change - already in result
            (Some(l), r) if l == r => {
                // Same change on both sides, already in result (from left)
                continue;
            }

            // Only right changed (left didn't touch this key)
            (None, Some(val)) => {
                mutations.push(Mutation::Upsert {
                    key: key.clone(),
                    val: val.clone(),
                });
            }
            (None, None) => {
                mutations.push(Mutation::Delete { key: key.clone() });
            }

            // Both changed but differently - conflict!
            (Some(_left_change), _right_change) => {
                let conflict = build_conflict(prolly, base, left, right, key)?;

                // Try to resolve the conflict
                if let Some(ref resolve) = resolver {
                    if let Some(resolved) = resolve(&conflict) {
                        mutations.push(Mutation::Upsert {
                            key: key.clone(),
                            val: resolved,
                        });
                        continue;
                    }
                }

                // No resolver or resolver returned None
                return Err(Error::Conflict(conflict));
            }
        }
    }

    if mutations.is_empty() {
        Ok(left.clone())
    } else {
        prolly.batch(left, mutations)
    }
}

/// Build a change map from a list of diffs.
///
/// Maps each key to its new value (Some for additions/changes, None for deletions).
pub fn build_change_map(diffs: &[Diff]) -> BTreeMap<Vec<u8>, Option<Vec<u8>>> {
    diffs
        .iter()
        .filter_map(|d| match d {
            Diff::Added { key, val } | Diff::Changed { key, new: val, .. } => {
                Some((key.clone(), Some(val.clone())))
            }
            Diff::Removed { key, .. } => Some((key.clone(), None)),
        })
        .collect()
}

/// Build conflict information for a key that has conflicting changes.
///
/// Retrieves the values from base, left, and right trees to provide
/// complete conflict information for resolution.
fn build_conflict<S: Store>(
    prolly: &Prolly<S>,
    base: &Tree,
    left: &Tree,
    right: &Tree,
    key: &[u8],
) -> Result<Conflict, Error> {
    let base_val = prolly.get(base, key)?;
    let left_val_actual = prolly.get(left, key)?.unwrap_or_default();
    let right_val_actual = prolly.get(right, key)?.unwrap_or_default();

    Ok(Conflict {
        key: key.to_vec(),
        base: base_val,
        left: left_val_actual,
        right: right_val_actual,
    })
}

#[cfg(test)]
mod tests {
    use super::super::config::Config;
    use super::super::store::MemStore;
    use super::*;

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

        assert_eq!(change_map.get(&b"a".to_vec()), Some(&Some(b"1".to_vec())));
        assert_eq!(change_map.get(&b"b".to_vec()), Some(&Some(b"new".to_vec())));
        assert_eq!(change_map.get(&b"c".to_vec()), Some(&None));
    }
}
