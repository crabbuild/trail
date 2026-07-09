# Plan 004: Stream root materialization in chunks

> **Executor instructions**: Follow this plan step by step. Run each verification command before moving to the next step. If a STOP condition occurs, stop and report instead of broadening scope. When done, update the status row for this plan in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 6bb6fa7..HEAD -- trail/src/db/storage/content.rs trail/src/db/storage/manifest.rs trail/src/db/lane/lifecycle.rs trail/src/db/record/checkout.rs`
>
> If any in-scope file changed since this plan was written, compare the "Current state" section against live code before proceeding. Treat a semantic mismatch as a STOP condition.

## Status

- **Priority**: P2
- **Effort**: L
- **Risk**: MED
- **Depends on**: `plans/001-skip-clean-cow-scan.md`, `plans/002-single-pass-workdir-manifest.md`
- **Category**: perf
- **Planned at**: commit `6bb6fa7`, 2026-06-27

## Why this matters

The current materialization APIs are convenient but load full roots and object bytes into maps. That inflates memory and duplicates work on large roots, especially when checkout loads both current and target roots to discover there is no diff. Streaming root entries in chunks keeps memory bounded and lets large workdir creation scale beyond the initial CoW wins.

## Current state

- `trail/src/db/storage/content.rs` exposes `load_root_files` around lines 124-139, returning a full `BTreeMap`.
- `materialize_entries_bytes` around lines 218-268 collects object IDs, fetches objects, and returns a second `BTreeMap<String, Vec<u8>>`.
- `trail/src/db/lane/lifecycle.rs` uses full-root loading for non-sparse lane materialization.
- `trail/src/db/record/checkout.rs` loads current root files and target root files around lines 38-48 before diffing them.
- `trail/src/db/storage/manifest.rs` already has streaming sorted-merge patterns in `diff_root_to_worktree_index` and `diff_root_to_disk_manifest`.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Format | `make fmt-check` | exit 0 |
| Check | `cargo check -p trail` | exit 0, no compiler errors |
| Focused tests | `cargo test -p trail streaming_root_materialization` | all new tests pass |
| Checkout regression | `cargo test -p trail checkout` | all matching checkout tests pass |
| Lane regression | `cargo test -p trail lane` | all matching lane tests pass |
| Scale benchmark | `make bench-cli-scale` | exit 0 and prints benchmark summary |

## Scope

**In scope**:

- `trail/src/db/storage/content.rs`
- `trail/src/db/storage/manifest.rs` only as an iterator exemplar or for shared helper extraction
- `trail/src/db/lane/lifecycle.rs`
- `trail/src/db/record/checkout.rs`
- materialization helper tests and lane/checkout tests
- benchmark docs only if this repo normally records benchmark deltas with performance work

**Out of scope**:

- removal of existing full-root map APIs
- storage object format changes
- compression/path interning
- daemon protocol changes

## Git workflow

- Branch: `advisor/004-stream-root-materialization`
- Commit style: conventional commits, for example `perf: stream root materialization in chunks`
- Do not push or open a PR unless explicitly asked.

## Steps

### Step 1: Add a chunked root-entry helper

In `content.rs`, add a streaming or chunked helper beside `load_root_files`. Suggested shape:

```rust
pub(crate) fn for_each_root_file_chunk(
    conn: &Connection,
    root_id: &Cid,
    chunk_size: usize,
    f: impl FnMut(BTreeMap<String, FileEntry>) -> Result<()>,
) -> Result<()>
```

Use the same sorted root iteration approach demonstrated in `manifest.rs`. Keep `load_root_files` available for existing callers.

**Verify**: `cargo check -p trail` -> exit 0.

### Step 2: Add chunked object-byte materialization

Add a helper that fetches object bytes for one chunk at a time. It should reuse the existing batched object lookup behavior and should not allocate bytes for files outside the current chunk.

**Verify**: `cargo test -p trail streaming_root_materialization_chunks` -> new chunk tests pass after they are added.

### Step 3: Add streaming materialization tests

Add tests with prefix `streaming_root_materialization` covering:

- chunked helper visits every file once in sorted order
- chunked object materialization matches existing full-root materialization on a small fixture
- a tiny chunk size forces multiple chunks

**Verify**: `cargo test -p trail streaming_root_materialization` -> all new tests pass.

### Step 4: Use streaming path for large lane materialization

In `lifecycle.rs`, use chunked materialization for large non-sparse roots, or for all non-sparse roots if the resulting code is simpler and benchmarks do not regress small cases.

Rules:

- Try clean-index CoW per chunk when plan 001 support exists.
- Fetch object bytes only for files that need object materialization.
- Accumulate destination stamps from plan 002.
- Write the clean manifest only after all chunks complete.
- On any chunk error, do not write a clean manifest.

**Verify**: `cargo test -p trail lane` -> all matching lane tests pass.

### Step 5: Add same-root checkout fast path

In `checkout.rs`, if `target.root_id == current.root_id` and a workdir output is requested, avoid loading both current and target roots just to compute an empty diff. Use clean-index CoW or streaming target materialization directly.

**Verify**: `cargo test -p trail checkout` -> all matching checkout tests pass.

### Step 6: Run the production gate and scale benchmark

Run:

```sh
make fmt-check
cargo check -p trail
cargo test -p trail
make bench-cli-scale-smoke
make bench-cli-scale
```

If runtime is acceptable, also run:

```sh
make bench-cli-scale-large
```

**Verify**: all required commands exit 0. Record wall-clock and peak memory changes for full materialized lane/worktree startup where the benchmark exposes them.

## Test plan

- New tests with prefix `streaming_root_materialization`.
- Existing checkout regression via `cargo test -p trail checkout`.
- Existing lane regression via `cargo test -p trail lane`.
- Full crate tests before DONE: `cargo test -p trail`.
- Scale benchmark before DONE: at least `make bench-cli-scale-smoke`, preferably `make bench-cli-scale`.

## Done criteria

- [ ] Large materialization no longer requires holding all file entries and all object bytes at once.
- [ ] Same-root checkout avoids duplicate full-root loading.
- [ ] Existing full-root APIs remain available and tested.
- [ ] Partial chunk failure cannot produce a clean manifest.
- [ ] Shared validation gate exits 0.
- [ ] `plans/README.md` status row is updated.

## STOP conditions

Stop and report if:

- The prolly iterator cannot provide stable sorted chunks without full-root loading.
- Chunked object fetch overhead causes unacceptable runtime regression after chunk-size tuning.
- The implementation requires removing current full-root APIs.
- Partial chunk failure can write or leave behind a clean manifest.

## Maintenance notes

- Reviewers should inspect memory behavior and benchmark attribution; this plan should improve peak memory without hiding regressions behind CoW wins.
- Future compression/path-interning work should benchmark after this lands so storage savings are measured separately from streaming savings.
- If chunk size becomes a tuning knob, keep the default internal until there is user evidence for exposing it.
