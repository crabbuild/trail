//! Batch builder for parallel tree construction
//!
//! The `BatchBuilder` enables efficient bulk loading of data into a Prolly tree
//! with parallel boundary detection and node creation using rayon.

use super::boundary::is_boundary_config;
use super::cid::Cid;
use super::config::Config;
use super::encoding::INIT_LEVEL;
use super::error::Error;
use super::node::Node;
use super::store::Store;
use super::tree::Tree;

use rayon::prelude::*;

/// Batch builder for parallel tree construction.
///
/// Enables efficient bulk loading of data into a Prolly tree with parallel
/// boundary detection and node creation using rayon.
///
/// # Example
/// ```
/// use prolly::{BatchBuilder, MemStore, Config};
/// use std::sync::Arc;
///
/// let store = Arc::new(MemStore::new());
/// let config = Config::default();
/// let mut builder = BatchBuilder::new(store, config);
///
/// builder.add(b"key1".to_vec(), b"val1".to_vec());
/// builder.add(b"key2".to_vec(), b"val2".to_vec());
/// builder.add(b"key3".to_vec(), b"val3".to_vec());
///
/// let tree = builder.build().unwrap();
/// ```
///
pub struct BatchBuilder<S: Store> {
    store: S,
    config: Config,
    /// Key-value pairs to insert (will be sorted before build)
    entries: Vec<(Vec<u8>, Vec<u8>)>,
}

impl<S: Store + Clone + Send + Sync> BatchBuilder<S>
where
    S::Error: Send + Sync,
{
    /// Create a new BatchBuilder with the given store and configuration.
    ///
    /// # Arguments
    /// * `store` - Storage backend implementing the `Store` trait
    /// * `config` - Tree configuration (chunking parameters, encoding, etc.)
    ///
    pub fn new(store: S, config: Config) -> Self {
        Self {
            store,
            config,
            entries: Vec::new(),
        }
    }

    /// Add a key-value pair to the builder.
    ///
    /// Entries will be sorted by key before building the tree.
    ///
    /// # Arguments
    /// * `key` - The key bytes
    /// * `val` - The value bytes
    ///
    pub fn add(&mut self, key: Vec<u8>, val: Vec<u8>) {
        self.entries.push((key, val));
    }

    /// Build the tree from the added entries using parallel chunking.
    ///
    /// This method:
    /// 1. Sorts entries by key
    /// 2. Partitions entries into chunks using parallel boundary detection
    /// 3. Creates leaf nodes in parallel
    /// 4. Builds internal nodes level by level
    ///
    /// # Returns
    /// * `Ok(Tree)` - The constructed tree
    /// * `Err(Error)` - If storage operations fail
    ///
    pub fn build(mut self) -> Result<Tree, Error> {
        // Handle empty case
        if self.entries.is_empty() {
            return Ok(Tree {
                root: None,
                config: self.config,
            });
        }

        // Sort entries by key
        self.entries.sort_by(|a, b| a.0.cmp(&b.0));

        // Parallel chunk building
        let chunks = self.parallel_chunk(&self.entries)?;

        // Build tree bottom-up
        self.build_from_chunks(chunks)
    }

    /// Partition entries into chunks in parallel using boundary detection.
    ///
    /// Uses rayon for parallel boundary detection and leaf node creation.
    ///
    /// # Arguments
    /// * `entries` - Sorted key-value pairs
    ///
    /// # Returns
    /// * `Ok(Vec<Cid>)` - CIDs of the created leaf nodes
    /// * `Err(Error)` - If storage operations fail
    ///
    fn parallel_chunk(&self, entries: &[(Vec<u8>, Vec<u8>)]) -> Result<Vec<Cid>, Error> {
        if entries.is_empty() {
            return Ok(vec![]);
        }

        // Find boundary points in parallel
        let boundaries: Vec<usize> = entries
            .par_iter()
            .enumerate()
            .filter_map(|(i, (k, v))| {
                // Check if this entry creates a boundary
                // count is i + 1 because we're considering entries 0..=i
                if is_boundary_config(&self.config, i + 1, k, v) {
                    Some(i)
                } else {
                    None
                }
            })
            .collect();

        // Build chunk ranges from boundaries
        let mut chunk_ranges: Vec<std::ops::RangeInclusive<usize>> = Vec::new();
        let mut start = 0;

        for end in &boundaries {
            if *end >= start {
                chunk_ranges.push(start..=*end);
                start = end + 1;
            }
        }

        // Add remaining entries as final chunk
        if start < entries.len() {
            chunk_ranges.push(start..=(entries.len() - 1));
        }

        // Handle edge case: if no boundaries found, create single chunk
        if chunk_ranges.is_empty() && !entries.is_empty() {
            chunk_ranges.push(0..=(entries.len() - 1));
        }

        // Create leaf nodes in parallel
        let store = &self.store;
        let config = &self.config;

        let cids: Result<Vec<Cid>, Error> = chunk_ranges
            .par_iter()
            .map(|range| {
                let mut node = Node::builder()
                    .leaf(true)
                    .level(INIT_LEVEL)
                    .min_chunk_size(config.min_chunk_size)
                    .max_chunk_size(config.max_chunk_size)
                    .chunking_factor(config.chunking_factor)
                    .hash_seed(config.hash_seed)
                    .encoding(config.encoding.clone())
                    .build();

                for i in range.clone() {
                    node.keys.push(entries[i].0.clone());
                    node.vals.push(entries[i].1.clone());
                }

                let bytes = node.to_bytes();
                let cid = Cid::from_bytes(&bytes);
                store
                    .put(cid.as_bytes(), &bytes)
                    .map_err(|e| Error::Store(Box::new(e)))?;
                Ok(cid)
            })
            .collect();

        cids
    }

    /// Build internal nodes from leaf CIDs, level by level.
    ///
    /// Constructs the tree bottom-up by creating internal nodes that
    /// reference the nodes from the previous level.
    ///
    /// # Arguments
    /// * `level_cids` - CIDs of nodes at the current (leaf) level
    ///
    /// # Returns
    /// * `Ok(Tree)` - The constructed tree with root
    /// * `Err(Error)` - If storage operations fail
    ///
    fn build_from_chunks(&self, mut level_cids: Vec<Cid>) -> Result<Tree, Error> {
        // Handle empty case
        if level_cids.is_empty() {
            return Ok(Tree {
                root: None,
                config: self.config.clone(),
            });
        }

        // Handle single node case - it becomes the root
        if level_cids.len() == 1 {
            return Ok(Tree {
                root: Some(level_cids.remove(0)),
                config: self.config.clone(),
            });
        }

        let mut level = INIT_LEVEL;

        // Build internal nodes level by level until we have a single root
        while level_cids.len() > 1 {
            level += 1;
            level_cids = self.build_level(level_cids, level)?;
        }

        Ok(Tree {
            root: level_cids.into_iter().next(),
            config: self.config.clone(),
        })
    }

    /// Build a single level of internal nodes from child CIDs.
    ///
    /// Creates internal nodes that reference the child nodes, using
    /// boundary detection to determine node boundaries.
    ///
    /// # Arguments
    /// * `child_cids` - CIDs of child nodes
    /// * `level` - The level number for the new internal nodes
    ///
    /// # Returns
    /// * `Ok(Vec<Cid>)` - CIDs of the created internal nodes
    /// * `Err(Error)` - If storage operations fail
    fn build_level(&self, child_cids: Vec<Cid>, level: u8) -> Result<Vec<Cid>, Error> {
        if child_cids.is_empty() {
            return Ok(vec![]);
        }

        // Load first key from each child for internal node keys
        let keys: Vec<Vec<u8>> = child_cids
            .par_iter()
            .map(|cid| {
                let bytes = self
                    .store
                    .get(cid.as_bytes())
                    .map_err(|e| Error::Store(Box::new(e)))?
                    .ok_or_else(|| Error::NotFound(cid.clone()))?;
                let node = Node::from_bytes(&bytes)?;
                Ok(node.keys.first().cloned().unwrap_or_default())
            })
            .collect::<Result<Vec<_>, Error>>()?;

        // Find boundaries for internal nodes
        let mut boundaries = Vec::new();
        for (i, (key, cid)) in keys.iter().zip(&child_cids).enumerate() {
            if is_boundary_config(&self.config, i + 1, key, cid.as_bytes()) {
                boundaries.push(i);
            }
        }

        // Build internal nodes
        let mut result = Vec::new();
        let mut start = 0;

        // Process each chunk defined by boundaries
        let boundary_iter = boundaries.iter().copied();
        let final_boundary = child_cids.len() - 1;

        // Collect all end points (boundaries + final index if not already included)
        let mut end_points: Vec<usize> = boundary_iter.collect();
        if end_points.is_empty() || *end_points.last().unwrap() != final_boundary {
            end_points.push(final_boundary);
        }

        for end in end_points {
            if start > end {
                continue;
            }

            let mut node = Node::builder()
                .leaf(false)
                .level(level)
                .min_chunk_size(self.config.min_chunk_size)
                .max_chunk_size(self.config.max_chunk_size)
                .chunking_factor(self.config.chunking_factor)
                .hash_seed(self.config.hash_seed)
                .encoding(self.config.encoding.clone())
                .build();

            for i in start..=end {
                node.keys.push(keys[i].clone());
                node.vals.push(child_cids[i].0.to_vec());
            }

            let bytes = node.to_bytes();
            let cid = Cid::from_bytes(&bytes);
            self.store
                .put(cid.as_bytes(), &bytes)
                .map_err(|e| Error::Store(Box::new(e)))?;
            result.push(cid);

            start = end + 1;
        }

        Ok(result)
    }
}
