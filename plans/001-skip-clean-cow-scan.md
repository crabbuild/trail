# Plan 001: Reuse clean worktree-index stamps before CoW

> **Executor instructions**: Follow this plan step by step. Run each verification command before moving to the next step. If a STOP condition occurs, stop and report instead of broadening scope. When done, update the status row for this plan in `plans/README.md`.
>
> **Drift check (run first)**: `git diff --stat 6bb6fa7..HEAD -- trail/src/db/storage/worktree_index.rs trail/src/db/lane/lifecycle.rs trail/src/db/record/checkout.rs trail/src/db/util/fs_cow.rs`
>
> If any in-scope file changed since this plan was written, compare the "Current state" section against live code before proceeding. Treat a semantic mismatch as a STOP condition.

## Status

- **Priority**: P1
- **Effort**: M
- **Risk**: MED
- **Depends on**: none
- **Category**: perf
- **Planned at**: commit `6bb6fa7`, 2026-06-27

## Why this matters

The first CoW implementation still pays for a full source workspace walk before cloning. On large clean worktrees, Trail already has a persisted worktree index and a baseline root, so scanning and hashing the source again is redundant. Reusing clean indexed stamps moves lane startup closer to "verify and clone the target files" instead of "walk the source, then clone".

## Current state

- `trail/src/db/lane/lifecycle.rs` owns lane workdir materialization. Around lines 256-301, `materialize_lane_workdir_at_paths_with_neighbors` loads root files, calls `workspace_file_stamps_if_entries_match`, tries `materialize_from_workspace_cow`, then writes a clean manifest.
- `trail/src/db/storage/worktree_index.rs` owns the persisted index. Around lines 330-348, `workspace_file_stamps_if_entries_match` calls `scan_worktree_manifest_indexed_with_stamps`, builds a disk manifest, diffs it against root entries, and returns stamps only when entries match.
- `trail/src/db/storage/worktree_index.rs` already has `worktree_index_baseline_root` and `set_worktree_index_baseline` around lines 610-624.
- `trail/src/db/util/fs_cow.rs` still validates source metadata before clone. Around lines 61-83, the stamped CoW helper compares expected file stamps before cloning, so indexed stamp reuse remains protected against source races.
- `trail/src/db/record/checkout.rs` has a same-root workdir branch around lines 52-78 that currently uses the same scan-based stamp helper before CoW.

## Commands you will need

| Purpose | Command | Expected on success |
| --- | --- | --- |
| Format | `make fmt-check` | exit 0 |
| Check | `cargo check -p trail` | exit 0, no compiler errors |
| Focused tests | `cargo test -p trail clean_index_stamp_reuse` | all new tests pass |
| Lane regression | `cargo test -p trail lane_spawn_supports_custom_and_configured_workdirs` | test passes |
| Smoke benchmark | `make bench-cli-scale-smoke` | exit 0 and prints benchmark summary |

## Scope

**In scope**:

- `trail/src/db/storage/worktree_index.rs`
- `trail/src/db/lane/lifecycle.rs`
- `trail/src/db/record/checkout.rs`
- `trail/src/db/util/fs_cow.rs` only for small signature changes if needed
- focused tests under existing `trail` test locations

**Out of scope**:

- `prolly/**`
- object storage formats
- workdir manifest format
- broad daemon protocol changes

## Git workflow

- Branch: `advisor/001-skip-clean-cow-scan`
- Commit style: conventional commits, for example `perf: reuse clean index stamps for CoW`
- Do not push or open a PR unless explicitly asked.

## Steps

### Step 1: Add an indexed stamp lookup helper

In `worktree_index.rs`, add a helper that reads indexed manifests and stamps for an exact path set without walking the filesystem. Suggested public-in-crate shape:

```rust
pub(crate) fn workspace_file_stamps_if_clean_index_matches(
    conn: &Connection,
    root_id: &Cid,
    files: &BTreeMap<String, FileEntry>,
) -> Result<Option<BTreeMap<String, WorktreeFileStamp>>>
```

Rules:

- Return `Ok(None)` if `worktree_index_baseline_root(conn)? != Some(root_id.clone())`.
- Query `worktree_file_index` in bounded chunks for the requested paths.
- Verify every requested path has a row.
- Verify indexed kind, executable bit, and content hash match the provided `FileEntry`.
- Return stamps only when all requested files match.
- Keep `workspace_file_stamps_if_entries_match` unchanged as the conservative fallback.

**Verify**: `cargo check -p trail` -> exit 0.

### Step 2: Add focused tests for the helper

Add tests named with the prefix `clean_index_stamp_reuse` covering:

- matching baseline root plus matching indexed rows returns all stamps
- missing indexed row returns `None`
- mismatched content hash returns `None`
- mismatched executable bit returns `None`
- mismatched baseline root returns `None`

Use existing `worktree_index.rs` test style if tests live in-module. If the repo keeps storage tests elsewhere, follow that local pattern.

**Verify**: `cargo test -p trail clean_index_stamp_reuse` -> all new tests pass.

### Step 3: Use the fast path in lane materialization

In `lifecycle.rs`, before calling `workspace_file_stamps_if_entries_match`, try `workspace_file_stamps_if_clean_index_matches`. If it returns `Some(stamps)`, pass those stamps to `materialize_from_workspace_cow`. If it returns `None`, keep the current scan-based path and fallback behavior.

Add debug or trace logging following the repo's existing style for:

- clean index hit
- clean index miss due to stale baseline
- clean index miss due to missing or mismatched rows

**Verify**: `cargo test -p trail lane_spawn_supports_custom_and_configured_workdirs` -> test passes.

### Step 4: Use the fast path in same-root checkout

In `checkout.rs`, for the branch where `target.root_id == current.root_id` and `workdir` is requested, try the clean-index stamp helper before the scan-based helper. Keep object materialization fallback intact.

**Verify**: `cargo test -p trail checkout` -> all matching checkout tests pass.

### Step 5: Run the production gate and benchmark

Run the shared gate:

```sh
make fmt-check
cargo check -p trail
cargo test -p trail
make bench-cli-scale-smoke
```

Then run a materialized smoke if the Makefile honors the variable:

```sh
TRAIL_SCALE_MATERIALIZED=1 make bench-cli-scale-smoke
```

**Verify**: all commands exit 0. The materialized benchmark should show lower clean lane/worktree startup time than the pre-change baseline, or the executor must report the numbers and reason.

## Test plan

- New helper tests with prefix `clean_index_stamp_reuse` in the storage-index test area.
- Existing lane spawn regression: `cargo test -p trail lane_spawn_supports_custom_and_configured_workdirs`.
- Existing checkout regression: `cargo test -p trail checkout`.
- Full crate tests before DONE: `cargo test -p trail`.

## Done criteria

- [ ] Clean indexed workspace stamps are used for CoW without a source workspace walk.
- [ ] Scan-based fallback remains intact for missing, stale, or mismatched index state.
- [ ] Clone-time source stamp validation remains in place.
- [ ] New tests cover hit and miss cases.
- [ ] Shared validation gate exits 0.
- [ ] `plans/README.md` status row is updated.

## STOP conditions

Stop and report if:

- `worktree_index_baseline_root` cannot reliably prove the root for ordinary clean workspaces.
- Implementing the helper requires changing the index table schema.
- The fast path requires removing clone-time metadata validation.
- A focused test fails twice after reasonable fixes.
- The change appears to require touching files outside the in-scope list.

## Maintenance notes

- Reviewers should scrutinize race handling between stamp lookup and clone. The clone-time check is the safety boundary.
- If future index schema changes add richer metadata, update this helper and its tests first.
- Sparse path CoW hydration should reuse this helper rather than adding another scanner.
