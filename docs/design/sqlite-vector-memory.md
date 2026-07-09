# SQLite Vector Memory Direction

Trail should build the agent memory layer on SQLite first. PGlite remains useful
as an experimental comparison point, but it should not be the default semantic
memory backend unless later benchmarks overturn the current storage results.

## Direction

- Store memory metadata and embedding rows in `.trail/index/trail.sqlite`.
- Keep structured memory rows as durable truth.
- Treat `sqlite-vec` `vec0` tables as the preferred local vector accelerator.
- Keep a portable exact-scan backend that stores embeddings as little-endian
  `f32` BLOBs in SQLite for fallback, verification, and baseline testing.
- Keep unsafe FFI registration small, reviewed, and isolated to SQLite extension
  setup.

The initial backend shape is:

```text
memory_items
  memory_ord INTEGER PRIMARY KEY AUTOINCREMENT
  memory_id TEXT UNIQUE
  scope_type TEXT
  scope_id TEXT
  kind TEXT
  path TEXT
  title TEXT
  body TEXT
  status TEXT
  source_ref TEXT
  source_change TEXT
  source_root TEXT
  metadata_json TEXT
  created_by TEXT
  updated_by TEXT
  created_at INTEGER
  updated_at INTEGER
  archived_at INTEGER

memory_embeddings
  memory_id TEXT PRIMARY KEY
  memory_ord INTEGER UNIQUE
  provider TEXT
  model TEXT
  dims INTEGER
  embedding BLOB
  embedding_hash TEXT

memory_embedding_indexes
  index_id TEXT PRIMARY KEY
  backend TEXT
  provider TEXT
  model TEXT
  dims INTEGER
  table_name TEXT

memory_vec_* -- sqlite-vec vec0 virtual table per provider/model/dims
  memory_ord INTEGER PRIMARY KEY
  embedding float[N] distance_metric=cosine
  scope_type TEXT
  scope_id TEXT
  kind TEXT
  path TEXT
  status TEXT

memory_revisions
  revision_id TEXT PRIMARY KEY
  memory_id TEXT
  version INTEGER
  operation TEXT
  body TEXT
  status TEXT
  source_change TEXT
  source_root TEXT
  embedding_hash TEXT
```

Search should combine SQLite filters with vector ranking:

```text
scope filter -> path/tag/kind/status filter -> vector ranking -> cited context packet
```

The implemented Rust API is:

```text
put_memory(MemoryPut) -> MemoryItem
archive_memory(memory_id, actor_id, MemoryVersionSource) -> MemoryItem
memory_item(memory_id) -> MemoryItem
search_memory(MemorySearch) -> Vec<MemorySearchResult>
memory_context_packet(MemorySearch) -> MemoryContextPacket
memory_revisions(memory_id) -> Vec<MemoryRevision>
memory_revision(memory_id, version) -> MemoryRevision
```

`MemorySearchBackend::Auto` uses `sqlite_vec0` when a provider/model/dimension
index exists and the requested filters are represented in the vector table. It
falls back to exact BLOB ranking for source-version filters and broad path-prefix
range scans. `MemorySearchBackend::SqliteVec` forces the extension path and
returns an error if no matching vector index exists.

## SQLite Vector Extension Policy

The preferred acceleration path is `sqlite-vec` compiled into the Rust binary.
The `sqlite-vec` crate exposes the `sqlite3_vec_init` C entrypoint and registers
with `rusqlite` through `sqlite3_auto_extension`. Unsafe code is acceptable for
this boundary when it is:

- centralized in one extension registration helper
- called before opening SQLite connections that need vector support
- covered by a smoke test that checks `vec_version()`
- not exposed to agent-supplied extension paths

Pin the crate to a stable buildable release. The current workspace uses
`sqlite-vec = "=0.1.9"` because `0.1.10-alpha.4` failed local compilation with a
missing bundled `sqlite-vec-diskann.c` include.

Acceptable acceleration options:

- Statically linked `sqlite-vec` through the Rust crate.
- A trusted loadable SQLite extension path configured by the user or installer
  for platforms where static linking is not practical.
- A future native SQLite vector extension if it becomes stable and portable.
- A future safe Rust API from the SQLite vector extension crate.

Not acceptable as the default:

- Requiring Node/PGlite for local memory search.
- Loading arbitrary extension paths from agent-supplied input.
- Treating vector rows as durable truth independent of structured memory rows.

## Benchmarks

Use `sqlite_vec0` as the target backend and `sqlite_exact_blob_scan` as the
control backend:

```sh
cargo bench -p trail --bench sqlite_vector_memory_bench
TRAIL_SQLITE_VECTOR_BACKEND=exact cargo bench -p trail --bench sqlite_vector_memory_bench
```

Useful environment variables:

```text
TRAIL_SQLITE_VECTOR_ROWS=100000
TRAIL_SQLITE_VECTOR_DIMS=768
TRAIL_SQLITE_VECTOR_QUERIES=50
TRAIL_SQLITE_VECTOR_TOP_K=20
TRAIL_SQLITE_VECTOR_KEEP_DB=1
TRAIL_SQLITE_VECTOR_BACKEND=sqlite_vec0
```

The benchmark intentionally measures:

- insertion of memory metadata plus embeddings
- scoped search
- path-prefix search
- context packet assembly
- SQLite database size

Compare `sqlite_vec0` against the exact backend before changing schema details,
metadata filters, or embedding dimensions.

Current 10k-row, 128-dimension benchmark result: `sqlite_vec0` is faster for
scope equality search and context packet assembly, but broad path-prefix range
search is slower than the exact BLOB scan. Treat equality-like filters
(`scope_type`, `scope_id`, status, path buckets) as the first `vec0` optimization
target. For arbitrary path-prefix range queries, either add a derived path bucket
metadata column and test it, or fall back to exact scan until the `vec0` query
shape is proven faster.

## Implementation Rule

The agent-facing memory API should not expose backend details. It should call a
`SemanticIndex` abstraction with at least these backends:

```text
sqlite_vec0
sqlite_exact_blob_scan
pglite_pgvector_experimental
```

Default to `sqlite_vec0` when the extension is compiled and smoke-tested.
Fallback to `sqlite_exact_blob_scan` when the extension cannot be loaded or when
the user explicitly asks for portable verification.
