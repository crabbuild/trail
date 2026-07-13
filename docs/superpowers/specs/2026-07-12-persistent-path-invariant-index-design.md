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
and clean Git mapping rows are copied from every distinct clean mapping for the
old root to each repaired root/maintenance change while preserving their Git
head, direction, and branch. Mapping discovery is root-wide rather than tied to
the old ref change, because historical imports and exports can establish valid
trust for the same immutable root under different changes. Duplicate source
tuples are collapsed before insertion.

Clean-state consumers may reuse a baseline whose root ID predates repair only
after loading both immutable roots and proving equal path-map root, file-ID-map
root, file count, and total text bytes. The exact-ID case remains the fast path;
missing or corrupt roots fail closed. This equivalence applies only to states
already proven clean. Dirty and overflow daemon snapshots are never promoted to
clean, and a live daemon snapshot is not rewritten or deleted by repair.

The maintenance operation row and its parent rows are indexed inside the same
authoritative SQLite transaction that advances refs and lane heads. Therefore a
process crash immediately after `COMMIT` still leaves operation lookup and
ancestry complete before a later broad index rebuild runs. Ref files remain
derived mirrors of SQLite refs and are reconciled on every rebuild.

Lane manifests and workspace checkpoint markers have a durable SQLite repair
queue (schema version 17). Each intent stores only ref name, repair kind, old
root, new root, and new change; filesystem paths are resolved again from the
current authoritative lane/view rows. Intents publish in the same transaction
as the ref repair, then drain after commit, at every rebuild, and on open after
an unlocked empty fast check plus a locked recheck. A new or already-retargeted
mirror, a missing scope/mirror, or conservative invalidation clears the intent;
an I/O failure leaves it for retry. Restore and backup verification use a
no-recovery database open so copied absolute paths are never followed. Restore
atomically rewrites lane workdirs and invalidates workspace-view/environment
state that the backup does not contain before draining intents and entering
normal recovery.

Legacy-root validation is two-pass and bounded by the largest distinct live
legacy root rather than the sum of all roots. The first pass validates one
root's key range, count, normalization, and fold collisions and retains only its
ID/count. After every root passes, the second pass re-ranges and builds one fold
tree at a time. A second rebuild after successful repair publishes no additional
root, operation, ref, or derived intent.

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
