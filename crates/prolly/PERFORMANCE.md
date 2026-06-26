# Prolly Performance Hardening

This note maps the paper `Accelerating Prolly Trees: Simplified Chunking for
Rapid Updates` to the Rust `prolly` crate implementation.

## Ideas

1. Bound update locality. Sequential inserts should touch only the right edge
   of the tree; random updates should stay limited to the affected chunk plus
   the ancestor path.
2. Treat the rightmost path as an anchor. Repeated append workloads should not
   rediscover the right edge from the root for every batch.
3. Batch contiguous inserts as a self-contained subtree. Construct new chunks
   in bulk, then splice them into the existing right edge.
4. Use subtree identity for sync and diff. Identical content-addressed subtrees
   are standalone comparison units and should be skipped or hydrated in bulk.
5. Make persistent stores first-class. Disk-backed workloads need atomic batch
   writes, ordered batch reads, and small sidecar hints to avoid read and write
   amplification.

## Implemented Hardening

- `append_batch` detects sorted append-only mutation batches and updates the
  rightmost path directly.
- The in-process rightmost-path cache avoids repeated right-edge traversal for
  append chains.
- Stores that support hints persist a compact rightmost-path hint keyed by the
  new root CID; fresh managers hydrate the hinted anchor path with one ordered
  batch read.
- Batch writes use `Store::batch_put` and SQLite transactions instead of generic
  per-node write loops.
- SQLite store supports ordered batch reads, batch writes, and atomic
  `batch_put_with_hint` for nodes plus rightmost-anchor hints.
- Diff and range-diff prune identical CIDs and range-disjoint child spans.
- Diff subtree collectors hydrate child nodes through `Prolly::load_many_ordered`
  so added, removed, and fallback subtrees use store-level ordered reads instead
  of one read per child.
- Node serialization has a packed format while retaining legacy CBOR reads.

## Current Bench Coverage

The `prolly_bench` harness covers:

- Point inserts, updates, deletes, and gets.
- Range scans and narrow range scans.
- Batch mutation paths, including append-only batches and append chains.
- Diff, stream diff, range diff, merge, and subtree add/remove diff.
- SQLite disk persistence for incremental writes, point updates, deletes,
  batch builds, reopen reads, batch mutations, append batches, append chains,
  fresh-manager append chains, and empty-to-full diff.

Example command:

```sh
PROLLY_BENCH_SCALE=5000 cargo bench -p prolly --bench prolly_bench --features sqlite
```

Recent scale-5000 disk results:

| Bench | ns/item | Approx items/sec |
| --- | ---: | ---: |
| `sqlite_disk_batch_builder_persist` | 1146 | 872k |
| `sqlite_disk_batch_mutations` | 9686 | 103k |
| `sqlite_disk_append_batch_persist` | 1677 | 596k |
| `sqlite_disk_append_batch_chain_persist` | 3727 | 268k |
| `sqlite_disk_append_batch_chain_new_manager_persist` | 4493 | 223k |
| `sqlite_disk_diff_empty_to_full` | 342 | 2.92M |

For large SQLite capacity checks, use the focused scale harness:

```sh
PROLLY_SQLITE_SCALE_STAGES=1000000,10000000,100000000 \
PROLLY_SQLITE_SCALE_BATCH=100000 \
PROLLY_SQLITE_SCALE_MAX_SECONDS=900 \
PROLLY_SQLITE_SCALE_MAX_DB_GB=70 \
cargo bench -p prolly --bench sqlite_scale_bench --features sqlite
```

Measured staged append results on the local machine:

| Target records | Total records | Total time ms | DB bytes | Bytes/record | Stage records/sec | Verified |
| ---: | ---: | ---: | ---: | ---: | ---: | :--- |
| 1,000,000 | 1,000,000 | 1554 | 135,456,400 | 135.46 | 643k | yes |
| 10,000,000 | 10,000,000 | 22,405 | 1,195,430,128 | 119.54 | 432k | yes |
| 100,000,000 | 100,000,000 | 606,434 | 11,721,597,960 | 117.22 | 154k | yes |

At the measured 100M footprint, a 1B-record SQLite database would require
roughly 117 GB before extra WAL, filesystem, and safety headroom. The local
volume had about 90 GiB free during this run, so 1B is disk-limited here.

## Remaining Safe Bets

1. Add a direct subtree-splice builder for large middle-range insertions where
   both neighboring boundaries are known.
2. Add a bounded-size node-cache policy. The current cache is intentionally
   simple and can grow with broad scans.
3. Add Criterion or divan benchmarks for lower-noise regression gates once API
   shape settles.
4. Evaluate SQLite prepared statement reuse across long-lived read batches if
   reopen/cold-manager workloads become dominant.
