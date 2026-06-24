//! Error types for Prolly Trees

use super::cid::Cid;

/// A mutation to apply to the tree
///
/// Represents a single operation in a batch mutation: either an upsert (insert or update)
/// or a delete operation.
///
#[derive(Clone, Debug, PartialEq)]
pub enum Mutation {
    /// Insert or update a key-value pair
    Upsert { key: Vec<u8>, val: Vec<u8> },
    /// Delete a key
    Delete { key: Vec<u8> },
}

impl Mutation {
    /// Get the key for this mutation
    ///
    pub fn key(&self) -> &[u8] {
        match self {
            Mutation::Upsert { key, .. } => key,
            Mutation::Delete { key } => key,
        }
    }

    /// Check if this is a delete mutation
    pub fn is_delete(&self) -> bool {
        matches!(self, Mutation::Delete { .. })
    }
}

/// Difference between two trees
///
/// Represents a single change between a base tree and another tree.
///
#[derive(Clone, Debug, PartialEq)]
pub enum Diff {
    /// Entry exists in the new tree but not in the base tree
    Added { key: Vec<u8>, val: Vec<u8> },
    /// Entry exists in the base tree but not in the new tree
    Removed { key: Vec<u8>, val: Vec<u8> },
    /// Entry exists in both trees but with different values
    Changed {
        key: Vec<u8>,
        old: Vec<u8>,
        new: Vec<u8>,
    },
}

/// Merge conflict information
///
/// Contains all the information needed to resolve a conflict during a three-way merge.
///
#[derive(Clone, Debug)]
pub struct Conflict {
    /// The key where the conflict occurred
    pub key: Vec<u8>,
    /// The value in the base tree (None if key didn't exist in base)
    pub base: Option<Vec<u8>>,
    /// The value in the left tree
    pub left: Vec<u8>,
    /// The value in the right tree
    pub right: Vec<u8>,
}

/// Conflict resolution strategy
///
/// A function that takes a conflict and returns an optional resolved value.
/// If `None` is returned, the merge will fail with a `Conflict` error.
///
///
/// # Example
/// ```
/// use prolly::Resolver;
///
/// // Always prefer the left value
/// let prefer_left: Resolver = Box::new(|conflict| Some(conflict.left.clone()));
///
/// // Always prefer the right value
/// let prefer_right: Resolver = Box::new(|conflict| Some(conflict.right.clone()));
///
/// // Concatenate values
/// let concat: Resolver = Box::new(|conflict| {
///     let mut result = conflict.left.clone();
///     result.extend(&conflict.right);
///     Some(result)
/// });
/// ```
pub type Resolver = Box<dyn Fn(&Conflict) -> Option<Vec<u8>>>;

/// Prolly tree errors
#[derive(Debug)]
pub enum Error {
    /// Node not found in store
    NotFound(Cid),
    /// Invalid node structure
    InvalidNode,
    /// Deserialization failed
    Deserialize(String),
    /// Storage error
    Store(Box<dyn std::error::Error + Send + Sync>),
    /// Merge conflict - occurs when both trees modify the same key differently
    /// and no resolver is provided or the resolver returns None
    ///
    Conflict(Conflict),
    /// Mutation buffer is full - adding a mutation would exceed the buffer size limit
    BufferFull,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::NotFound(cid) => write!(f, "node not found: {:?}", cid),
            Error::InvalidNode => write!(f, "invalid node structure"),
            Error::Deserialize(e) => write!(f, "deserialize error: {}", e),
            Error::Store(e) => write!(f, "storage error: {}", e),
            Error::Conflict(c) => write!(f, "merge conflict at key: {:?}", c.key),
            Error::BufferFull => write!(f, "mutation buffer is full"),
        }
    }
}

impl std::error::Error for Error {}
