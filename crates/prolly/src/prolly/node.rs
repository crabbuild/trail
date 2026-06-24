//! Node structure and builder for Prolly Trees

use serde::{Deserialize, Serialize};

use super::cid::Cid;
use super::encoding::{
    Encoding, DEFAULT_CHUNKING_FACTOR, DEFAULT_HASH_SEED, DEFAULT_MAX_CHUNK_SIZE,
    DEFAULT_MIN_CHUNK_SIZE, INIT_LEVEL,
};
use super::error::Error;

/// A node in the Prolly Tree
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Node {
    /// Keys (sorted, lexicographic byte order)
    pub keys: Vec<Vec<u8>>,
    /// Values: raw bytes for leaves, CIDs for internal nodes
    pub vals: Vec<Vec<u8>>,
    /// Leaf node (true) or internal node (false)
    pub leaf: bool,
    /// Tree level (0 = leaf level)
    pub level: u8,
    /// Minimum entries before considering boundary
    pub min_chunk_size: usize,
    /// Maximum entries in a node
    pub max_chunk_size: usize,
    /// Chunking factor (higher = larger nodes)
    pub chunking_factor: u32,
    /// Hash seed for boundary detection
    pub hash_seed: u64,
    /// Value encoding type
    pub encoding: Encoding,
}

impl Default for Node {
    fn default() -> Self {
        Self {
            keys: Vec::new(),
            vals: Vec::new(),
            leaf: true,
            level: INIT_LEVEL,
            min_chunk_size: DEFAULT_MIN_CHUNK_SIZE,
            max_chunk_size: DEFAULT_MAX_CHUNK_SIZE,
            chunking_factor: DEFAULT_CHUNKING_FACTOR,
            hash_seed: DEFAULT_HASH_SEED,
            encoding: Encoding::Raw,
        }
    }
}

impl Node {
    /// Create a new leaf node with default settings
    pub fn new_leaf() -> Self {
        Self::default()
    }

    /// Create a new internal node at the specified level
    pub fn new_internal(level: u8) -> Self {
        Self {
            leaf: false,
            level,
            ..Default::default()
        }
    }

    /// Create a builder for constructing a Node
    pub fn builder() -> NodeBuilder {
        NodeBuilder::default()
    }

    /// Get the number of keys in this node
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Check if this node is empty
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Binary search for key index
    /// Returns Ok(index) if found, Err(index) for insertion point
    pub fn search(&self, key: &[u8]) -> Result<usize, usize> {
        self.keys.binary_search_by(|k| k.as_slice().cmp(key))
    }

    /// Serialize to bytes (deterministic CBOR)
    pub fn to_bytes(&self) -> Vec<u8> {
        serde_cbor::ser::to_vec_packed(self).expect("serialization should not fail")
    }

    /// Deserialize from bytes
    pub fn from_bytes(data: &[u8]) -> Result<Self, Error> {
        serde_cbor::from_slice(data).map_err(|e| Error::Deserialize(e.to_string()))
    }

    /// Compute CID of this node
    pub fn cid(&self) -> Cid {
        Cid::from_bytes(&self.to_bytes())
    }
}

/// Builder pattern for Node construction
#[derive(Default)]
pub struct NodeBuilder {
    keys: Vec<Vec<u8>>,
    vals: Vec<Vec<u8>>,
    leaf: bool,
    level: u8,
    min_chunk_size: usize,
    max_chunk_size: usize,
    chunking_factor: u32,
    hash_seed: u64,
    encoding: Encoding,
}

impl NodeBuilder {
    /// Create a new NodeBuilder with default values
    pub fn new() -> Self {
        Self {
            leaf: true,
            level: INIT_LEVEL,
            min_chunk_size: DEFAULT_MIN_CHUNK_SIZE,
            max_chunk_size: DEFAULT_MAX_CHUNK_SIZE,
            chunking_factor: DEFAULT_CHUNKING_FACTOR,
            hash_seed: DEFAULT_HASH_SEED,
            encoding: Encoding::Raw,
            keys: Vec::new(),
            vals: Vec::new(),
        }
    }

    /// Set the keys
    pub fn keys(mut self, keys: Vec<Vec<u8>>) -> Self {
        self.keys = keys;
        self
    }

    /// Set the values
    pub fn vals(mut self, vals: Vec<Vec<u8>>) -> Self {
        self.vals = vals;
        self
    }

    /// Set whether this is a leaf node
    pub fn leaf(mut self, leaf: bool) -> Self {
        self.leaf = leaf;
        self
    }

    /// Set the tree level
    pub fn level(mut self, level: u8) -> Self {
        self.level = level;
        self
    }

    /// Set the minimum chunk size
    pub fn min_chunk_size(mut self, size: usize) -> Self {
        self.min_chunk_size = size;
        self
    }

    /// Set the maximum chunk size
    pub fn max_chunk_size(mut self, size: usize) -> Self {
        self.max_chunk_size = size;
        self
    }

    /// Set the chunking factor
    pub fn chunking_factor(mut self, factor: u32) -> Self {
        self.chunking_factor = factor;
        self
    }

    /// Set the hash seed
    pub fn hash_seed(mut self, seed: u64) -> Self {
        self.hash_seed = seed;
        self
    }

    /// Set the encoding type
    pub fn encoding(mut self, encoding: Encoding) -> Self {
        self.encoding = encoding;
        self
    }

    /// Build the Node
    pub fn build(self) -> Node {
        Node {
            keys: self.keys,
            vals: self.vals,
            leaf: self.leaf,
            level: self.level,
            min_chunk_size: self.min_chunk_size,
            max_chunk_size: self.max_chunk_size,
            chunking_factor: self.chunking_factor,
            hash_seed: self.hash_seed,
            encoding: self.encoding,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_leaf() {
        let node = Node::new_leaf();
        assert!(node.leaf);
        assert_eq!(node.level, INIT_LEVEL);
        assert!(node.is_empty());
    }

    #[test]
    fn test_new_internal() {
        let node = Node::new_internal(2);
        assert!(!node.leaf);
        assert_eq!(node.level, 2);
    }

    #[test]
    fn test_builder() {
        let node = Node::builder()
            .keys(vec![b"key1".to_vec(), b"key2".to_vec()])
            .vals(vec![b"val1".to_vec(), b"val2".to_vec()])
            .leaf(true)
            .level(0)
            .min_chunk_size(2)
            .max_chunk_size(100)
            .chunking_factor(64)
            .hash_seed(42)
            .encoding(Encoding::Cbor)
            .build();

        assert_eq!(node.len(), 2);
        assert!(node.leaf);
        assert_eq!(node.level, 0);
        assert_eq!(node.min_chunk_size, 2);
        assert_eq!(node.max_chunk_size, 100);
        assert_eq!(node.chunking_factor, 64);
        assert_eq!(node.hash_seed, 42);
        assert_eq!(node.encoding, Encoding::Cbor);
    }

    #[test]
    fn test_search() {
        let node = Node::builder()
            .keys(vec![b"a".to_vec(), b"c".to_vec(), b"e".to_vec()])
            .vals(vec![b"1".to_vec(), b"2".to_vec(), b"3".to_vec()])
            .build();

        assert_eq!(node.search(b"a"), Ok(0));
        assert_eq!(node.search(b"c"), Ok(1));
        assert_eq!(node.search(b"e"), Ok(2));
        assert_eq!(node.search(b"b"), Err(1));
        assert_eq!(node.search(b"d"), Err(2));
    }

    #[test]
    fn test_len_is_empty() {
        let empty = Node::new_leaf();
        assert!(empty.is_empty());
        assert_eq!(empty.len(), 0);

        let node = Node::builder()
            .keys(vec![b"key".to_vec()])
            .vals(vec![b"val".to_vec()])
            .build();
        assert!(!node.is_empty());
        assert_eq!(node.len(), 1);
    }

    #[test]
    fn test_serialization_roundtrip() {
        let node = Node::builder()
            .keys(vec![b"key1".to_vec(), b"key2".to_vec()])
            .vals(vec![b"val1".to_vec(), b"val2".to_vec()])
            .leaf(true)
            .level(0)
            .build();

        let bytes = node.to_bytes();
        let restored = Node::from_bytes(&bytes).unwrap();
        assert_eq!(node, restored);
    }

    #[test]
    fn packed_serialization_reads_legacy_cbor_and_reduces_size() {
        let node = Node::builder()
            .keys(vec![b"key1".to_vec(), b"key2".to_vec(), b"key3".to_vec()])
            .vals(vec![b"val1".to_vec(), b"val2".to_vec(), b"val3".to_vec()])
            .leaf(true)
            .level(0)
            .min_chunk_size(2)
            .max_chunk_size(128)
            .chunking_factor(64)
            .hash_seed(42)
            .encoding(Encoding::Raw)
            .build();

        let legacy_bytes = serde_cbor::to_vec(&node).unwrap();
        let packed_bytes = node.to_bytes();

        assert_eq!(Node::from_bytes(&legacy_bytes).unwrap(), node);
        assert_eq!(Node::from_bytes(&packed_bytes).unwrap(), node);
        assert!(packed_bytes.len() < legacy_bytes.len());
    }

    #[test]
    fn test_cid_deterministic() {
        let node1 = Node::builder()
            .keys(vec![b"key".to_vec()])
            .vals(vec![b"val".to_vec()])
            .build();

        let node2 = Node::builder()
            .keys(vec![b"key".to_vec()])
            .vals(vec![b"val".to_vec()])
            .build();

        assert_eq!(node1.cid(), node2.cid());
    }
}
