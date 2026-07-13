# Persistent Path-Invariant Index Design

**Status:** Approved as the second production performance slice

## Objective

Make case-insensitive path-safety validation proportional to touched paths `k`,
not root size `N`. Structured patches, materialized records, Git imports, lane
merges, and incremental root builders must not load every root path merely to
prove that a small mutation introduces no case-fold collision.

## Current bottleneck

`ensure_patch_final_root_paths_safe`,
`ensure_record_final_root_paths_safe_from_summaries`, and several incremental
root builders call `load_root_paths()` and rebuild an in-memory folded-path map.
At 100k/1M paths this adds an `O(N)` read to otherwise incremental operations.

## Root schema

Add an optional `case_fold_map_root` to `WorktreeRoot`. Its Prolly map stores:

```text
NFKC(lowercase(path)), NFC-normalized -> canonical path bytes
```

The field is Serde-defaulted for old roots. New non-empty roots carry the index
root. Because an empty Prolly tree has no root CID, `case_fold_map_root=None`
is also a valid empty index exactly when `file_count=0`; no path scan is needed
to establish that state. The index uses the same persistent store/configuration
as the path and file-ID maps, so unchanged nodes are content-addressed and
shared.

## Mutation contract

For an indexed root, validation applies removals to an overlay first, then
checks each addition's folded key against the overlay and persistent tree.
Distinct canonical paths at the same folded key return the existing
`InvalidPath` collision error. On success, one Prolly mutation batch produces
the next case-fold tree. Work is `O(k log N)` plus affected-node writes.

Every root-construction path must keep the path map, file-ID map, and case-fold
map in sync. Full builders construct all three in one pass. Incremental builders
derive removals/additions from the same touched-path set used for the path map.

## Legacy roots and repair

An old non-empty root without the index is not silently scanned on a hot
mutation path. Hot callers return stable `PATH_INDEX_REQUIRED` with the
recovery command `trail index rebuild`. A root with no index CID and
`file_count=0` is already a valid empty index and may be mutated directly. The
existing rebuild operation backfills the current root/index state.
Compatibility read/materialization operations may still read old roots; only
mutation safety requires the index.

Repair covers every mutable live `refs/branches/*` and `refs/lanes/*` head in
one command, not only the checked-out branch. Distinct refs that share a legacy
root share one rebuilt fold tree and one equivalent immutable root. Historical
roots remain unchanged. Each repaired ref advances by CAS through its own
auditable `ManualCheckpoint` maintenance operation with the old head as parent,
the old/new equivalent roots as `before_root`/`after_root`, and no visible file
changes. All roots and lane/Git metadata are preflighted before any ref is
published; authoritative SQLite ref/lane/checkpoint/mapping updates commit as a
unit. Content-addressed nodes or objects created during a failed preflight may
remain unreachable and are reclaimed by normal GC, but no live ref advances.

Derived baselines follow the equivalent root identity without touching visible
files. A clean checked-out worktree baseline is retargeted, clean lane manifests
and workspace checkpoint markers are retargeted or conservatively invalidated,
and clean Git mapping rows are copied from the old root/change to each repaired
root/maintenance change while preserving their recorded Git head, direction,
and branch. Ref files are derived mirrors of SQLite refs and are reconciled on
every rebuild, making an interrupted mirror write retryable. A second rebuild
after successful repair publishes no additional root, operation, or ref.

## Structural evidence

Expose path-invariant metrics on patch/record scale scenarios:

- `case_fold_index_mode=indexed`
- `case_fold_index_lookups<=k`
- `full_root_path_loads=0`

Add 1k CI and 100k/1M scheduled gates. Collision, rename, delete-then-add,
Unicode compatibility, legacy-root recovery, and index corruption tests are
required before rollout.

## Rollout

1. Add the backward-compatible root field and full-build index construction.
2. Add indexed lookup/mutation helpers and stable legacy-root error.
3. Replace hot full-path validators and incremental builders.
4. Backfill through `trail index rebuild` and add scale counters/gates.
5. Run full tests plus 100k/1M patch/record acceptance.
