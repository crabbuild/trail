# Prolly Tree

`prolly` provides content-addressed prolly tree storage primitives for CrabDB.
It is an immutable, ordered key-value index over byte keys and byte values, with
stable content-derived structure for efficient structural sharing, diff, merge,
and bulk loading.

At the API boundary, a `Tree` is a small persistent handle:

- `root: Option<Cid>` points at the content-addressed root node.
- `config: Config` records the chunking and encoding parameters used by the tree.

The actual nodes live in a pluggable `Store`. Operations clone and rewrite only
the affected path or subtrees, write new content-addressed nodes, and return a
new `Tree` handle.

## Architecture

![Prolly tree architecture](diagram/prolly-tree-architecture.svg)

The same diagram is also rendered as
[`diagram/prolly-tree-architecture@2x.png`](diagram/prolly-tree-architecture@2x.png)
for contexts that prefer raster images.

## What This Crate Gives You

- Ordered byte-key lookup with lexicographic key ordering.
- Immutable updates: `put`, `delete`, and `batch` return a new `Tree`.
- Content-addressed nodes: each node CID is the SHA-256 hash of deterministic
  node bytes.
- Deterministic content-defined chunking using xxHash64 boundary checks.
- Structural sharing between versions because unchanged nodes keep the same CID.
- Efficient diff and range diff by pruning equal CIDs and disjoint child spans.
- Three-way merge with conflict resolver support.
- CRDT-style conflict-free merge strategies.
- Lazy range iteration and cursor-based traversal.
- Batch mutation paths for sorted, grouped, append-heavy, and multi-leaf writes.
- Parallel bulk builders for large initial trees.
- Pluggable storage through the `Store` trait, with memory, SQLite, and optional
  RocksDB implementations.
- Tree statistics for inspecting shape, fill factor, fanout, and serialized size.

## Quick Start

```rust
use prolly::{Config, MemStore, Prolly};

let store = MemStore::new();
let prolly = Prolly::new(store, Config::default());

let tree = prolly.create();
let tree = prolly
    .put(&tree, b"name".to_vec(), b"Alice".to_vec())
    .unwrap();

let value = prolly.get(&tree, b"name").unwrap();
assert_eq!(value, Some(b"Alice".to_vec()));

let tree = prolly.delete(&tree, b"name").unwrap();
assert!(prolly.get(&tree, b"name").unwrap().is_none());
```

All update APIs are persistent. The old `Tree` handle remains valid as long as
the store still contains the nodes it references.

## Core Data Model

### `Tree`

`Tree` is the durable handle returned to callers. It does not own node data.
It only records the root CID and the `Config` used to build the tree.

An empty tree has `root == None`. A non-empty tree has `root == Some(Cid)`.

### `Cid`

`Cid` is a 32-byte SHA-256 digest of serialized node bytes. Equal node content
produces the same CID, which gives the tree Merkle-style identity:

- Equal roots mean equal trees.
- Equal child CIDs let diff skip entire subtrees.
- Rewritten paths can share all untouched sibling subtrees.

### `Node`

A `Node` stores sorted `keys` and parallel `vals`.

For a leaf node:

```text
keys = [k1, k2, k3]
vals = [v1, v2, v3]        // raw user value bytes
```

For an internal node:

```text
keys = [first_key_child_1, first_key_child_2, ...]
vals = [child_cid_1, child_cid_2, ...]  // 32-byte CID bytes
```

Important node fields:

- `leaf`: whether values are raw data or child CIDs.
- `level`: `0` for leaves, increasing toward the root.
- `min_chunk_size`: entries before hash boundaries can split a chunk.
- `max_chunk_size`: hard upper bound that forces splitting.
- `chunking_factor`: average-boundary tuning; higher means larger chunks.
- `hash_seed`: deterministic seed for boundary placement.
- `encoding`: value encoding metadata (`Raw`, `Cbor`, `Json`, or `Custom`).

Nodes serialize to a compact deterministic format with the `CRAB` magic header.
Legacy CBOR node bytes are still readable.

## Content-Defined Chunking

Prolly trees use content-defined chunk boundaries rather than fixed B-tree split
points. Boundary placement is stable for the same content and config.

Boundary rules:

1. If the current chunk is below `min_chunk_size`, do not split.
2. If the current chunk is at or beyond `max_chunk_size`, force a split.
3. Otherwise hash `key || value` with xxHash64 and compare the lower 32 bits to
   `u32::MAX / chunking_factor`.

The default `chunking_factor` is `128`, so the expected boundary probability is
roughly `1 / 128`. Lower factors produce more, smaller chunks. Higher factors
produce fewer, larger chunks.

This is the key property that makes prolly trees good for local-first storage
and versioned indexes: small edits usually rewrite a local leaf and ancestor
path, while unchanged content keeps identical CIDs.

## Read Path

`get(&tree, key)` performs a root-to-leaf search:

1. Load `tree.root` from the `Store`.
2. Binary-search the current node's sorted `keys`.
3. In internal nodes, descend to the child whose span can contain the key.
4. In a leaf, return the exact key's value or `None`.

The expected complexity is `O(log n)` node visits.

Range APIs use similar positioning:

- `range(&tree, start, end)` returns a lazy `RangeIter`.
- `cursor(&tree, key)` positions a cursor near a key.
- `range_cursor(&tree, start, end)` uses cursor traversal for bounded scans.

Range iteration performs an initial seek and then advances across leaves in
sorted key order.

## Write Path

`put` and `delete` are immutable operations.

Single-key writes:

1. `find_path` walks from root to the target leaf.
2. The leaf is cloned and updated.
3. `rebalance_with_collector` splits, merges, or propagates changes upward.
4. New or changed nodes are collected.
5. The collector flushes node bytes through the store.
6. The method returns a new `Tree` with the new root CID.

The original tree remains valid and shares any unchanged subtrees.

Append-heavy single-key writes can use the rightmost-path fast path when the key
belongs after the current right edge.

## Batch Mutation Path

`batch(&tree, mutations)` is the default API for multiple updates:

```rust
use prolly::{Config, MemStore, Mutation, Prolly};

let store = MemStore::new();
let prolly = Prolly::new(store, Config::default());
let tree = prolly.create();

let mutations = vec![
    Mutation::Upsert {
        key: b"a".to_vec(),
        val: b"1".to_vec(),
    },
    Mutation::Upsert {
        key: b"b".to_vec(),
        val: b"2".to_vec(),
    },
    Mutation::Delete {
        key: b"old".to_vec(),
    },
];

let tree = prolly.batch(&tree, mutations).unwrap();
```

Batch processing:

- Sorts mutations by key.
- Deduplicates duplicate keys with last-write-wins semantics.
- Detects append-only batches and updates the rightmost path directly when
  possible.
- Groups mutations by target leaf.
- Can prefetch leaves through `Store::batch_get_ordered`.
- Applies grouped mutations with either a two-pointer merge or binary search.
- Rebuilds affected parents and flushes nodes atomically through store batch
  writes when supported.

For explicit tuning, use `BatchWriter` and `BatchWriterConfig`:

```rust
use prolly::{BatchWriter, BatchWriterConfig};

let writer = BatchWriter::with_config(
    BatchWriterConfig::new()
        .with_prefetch(true)
        .with_optimized_merge(true)
        .with_bottom_up_rebuild(true),
);
```

The default config enables prefetch, optimized merge, and deferred rebalancing.
Bottom-up rebuild is available for workloads that touch many leaves.

## Bulk Building

Use `BatchBuilder` when you have many unsorted entries and want to build a fresh
tree:

```rust
use prolly::{BatchBuilder, Config, MemStore};
use std::sync::Arc;

let store = Arc::new(MemStore::new());
let mut builder = BatchBuilder::new(store, Config::default());

for i in 0..1000 {
    builder.add(
        format!("key-{i:04}").into_bytes(),
        format!("value-{i}").into_bytes(),
    );
}

let tree = builder.build().unwrap();
```

`BatchBuilder` sorts entries, computes hash-boundary predicates in parallel with
Rayon, writes leaf nodes in batches, and then builds internal levels bottom-up.

Use `SortedBatchBuilder` when the input is already sorted by key. It can stream
leaf construction without retaining every key-value pair in memory.

## Diff, Range Diff, and Merge

Diff APIs compare tree structure before falling back to leaf-level comparison:

- `diff(&base, &other)` returns collected `Diff` entries.
- `range_diff(&base, &other, start, end)` prunes subtrees outside a half-open
  key range.
- `diff_cursor(&base, &other)` and `stream_diff(&base, &other)` stream changes.

Fast paths:

- Same root CID returns no changes in `O(1)`.
- Equal child CIDs skip whole subtrees.
- Matching child spans recurse structurally.
- Divergent boundaries fall back to ordered collection and comparison for the
  affected subtree.

`merge(&base, &left, &right, resolver)` performs a three-way merge. It detects
conflicts when both sides change the same key differently. A resolver can return
the chosen value, or the merge returns `Error::Conflict`.

`crdt_merge` uses `CrdtConfig` for automatic conflict-free merge behavior such
as last-writer-wins, multi-value preservation, or a custom merge function.

## Storage Backends

The `Store` trait is intentionally small and content-addressed. Store keys are
CID bytes and values are serialized node bytes.

Required methods:

- `get`
- `put`
- `delete`
- `batch`

Optional optimized methods:

- `batch_get`
- `batch_get_ordered`
- `batch_put`
- `supports_hints`
- `get_hint`
- `put_hint`
- `batch_put_with_hint`

Built-in stores:

- `MemStore`: in-memory store for tests and lightweight use.
- `SqliteStore`: persistent SQLite backend behind the `sqlite` feature.
- `RocksDBStore`: optional RocksDB backend behind the `rocksdb` feature.

Feature flags:

```sh
cargo test -p prolly
cargo test -p prolly --features sqlite
cargo test -p prolly --features rocksdb
```

## Caches and Hints

`Prolly<S>` maintains two in-process caches:

- `node_cache`: immutable nodes keyed by CID.
- `rightmost_path_cache`: the known right edge for append-heavy workloads.

Stores may also persist performance hints. SQLite stores a rightmost-path hint
alongside node writes so a fresh `Prolly` manager can hydrate the append anchor
with ordered batch reads. Hints are never required for correctness; callers must
always have a normal traversal fallback.

Use `clear_cache()` after tests or external store maintenance that intentionally
mutates the backing store outside the `Prolly` API. Use `cache_len()` to inspect
the current node-cache size.

## Configuration

```rust
use prolly::{Config, Encoding};

let config = Config::builder()
    .min_chunk_size(4)
    .max_chunk_size(1024)
    .chunking_factor(128)
    .hash_seed(42)
    .encoding(Encoding::Raw)
    .build();
```

Tuning guide:

| Setting | Effect |
| --- | --- |
| `min_chunk_size` | Prevents tiny chunks by disabling boundary checks until the chunk is large enough. |
| `max_chunk_size` | Forces a split and bounds worst-case node size. |
| `chunking_factor` | Higher values create fewer boundaries and larger average nodes. |
| `hash_seed` | Changes deterministic boundary placement for the same content. |
| `encoding` | Records value encoding metadata on nodes. |

For durable stores, larger chunks generally reduce node count and I/O, while
smaller chunks can improve edit locality and diff granularity.

## Statistics

`collect_stats(&tree)` traverses the tree and returns `TreeStats`, including:

- node, leaf, and internal-node counts;
- tree height;
- total key-value pairs;
- serialized size metrics;
- entries per level;
- fanout and fill factor;
- key and value size distribution.

This is useful when tuning chunking parameters or comparing storage backends.

## Complexity

Approximate costs:

| Operation | Cost |
| --- | --- |
| `get` | `O(log n)` node visits |
| `put` / `delete` | `O(log n)` path rewrite plus rebalancing |
| `range` | `O(log n + k)` for `k` yielded entries |
| `batch` | sort and group mutations, then rewrite affected leaves and ancestors |
| `diff` | `O(changed subtrees)` when boundaries align; local ordered fallback otherwise |
| same-root `diff` | `O(1)` |
| `merge` | diffs plus batch application of non-conflicting changes |

## Testing and Benchmarks

Run crate tests:

```sh
cargo test -p prolly
```

Run with SQLite support:

```sh
cargo test -p prolly --features sqlite
```

Run the main benchmark harness:

```sh
PROLLY_BENCH_SCALE=5000 cargo bench -p prolly --bench prolly_bench --features sqlite
```

Run the focused SQLite scale harness:

```sh
PROLLY_SQLITE_SCALE_STAGES=1000000,10000000 \
PROLLY_SQLITE_SCALE_BATCH=100000 \
cargo bench -p prolly --bench sqlite_scale_bench --features sqlite
```

See [`PERFORMANCE.md`](PERFORMANCE.md) for the performance hardening notes,
current benchmark coverage, and measured SQLite scale results.

## Module Map

| Module | Responsibility |
| --- | --- |
| `tree.rs` | Persistent `Tree` handle. |
| `node.rs` | Node layout, compact serialization, node CID calculation. |
| `cid.rs` | SHA-256 content identifier. |
| `config.rs` | Chunking and encoding configuration. |
| `boundary.rs` | xxHash64 content-defined boundary detection. |
| `rebalance.rs` | Splitting, merging, parent propagation, root changes. |
| `batch.rs` | Batch mutation processing, append paths, collectors, rebuild helpers. |
| `builder.rs` | Parallel and sorted bulk tree construction. |
| `range.rs` | Lazy range iteration. |
| `cursor.rs` | Cursor traversal and streaming diff cursor. |
| `diff.rs` | Structural diff, range diff, and three-way merge. |
| `crdt.rs` | Conflict-free merge strategies. |
| `parallel.rs` | Parallel batch/rebalance interfaces. |
| `streaming.rs` | Streaming differ trait and default implementation. |
| `stats.rs` | Tree shape and size metrics. |
| `store/` | Storage trait and backend implementations. |

## When To Use It

Use this crate when you need an ordered map that can cheaply keep multiple
versions, diff them, merge them, or persist them into a content-addressed store.
It is a good fit for local-first databases, versioned metadata indexes,
replication/sync state, and systems like CrabDB that need stable structural
identity between snapshots.
