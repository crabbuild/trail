# Storage, Indexes, and Backups

Backups include cache decisions and private-output publication journals, but
not mounted views, active environment generations, or performance-cache layer
bytes. Restore retains publication attempts as `recovered` provenance, removes
their attachable layer/generation authority, and requires the lane to prepare a
fresh view and generation. A restored workspace validates that closed state
before reporting healthy.

Trail stores durable workspace state under `.trail` and maintains derived indexes for fast history and provenance queries.

## SQLite and Prolly Storage

The Trail index lives under:

```text
.trail/index/trail.sqlite
```

The `prolly` crate is re-exported from the `trail` crate and is used for map roots and content-addressed tree structures.

Prolly tree nodes are stored in SQLite alongside Trail metadata for refs,
operations, derived indexes, and workspace bookkeeping. The schema records the
node encoding used by `prolly-store-sqlite` 0.4, allowing raw and compressed
nodes to coexist without changing their content identities.

## Derived Indexes

Indexes support:

- File history.
- Line history.
- Message lookup.
- Session and agent operation lookup.
- Worktree file status acceleration.

Rebuild derived indexes with:

```sh
trail index rebuild
```

Use rich text hydration during rebuild when needed:

```sh
trail index rebuild --rich-text
```

Refresh the worktree file index:

```sh
trail index watch --once
```

## Health and Integrity

Use:

```sh
trail doctor
trail fsck
```

`doctor` checks operational readiness, schema version, current branch,
`.trailignore` defaults, runtime integration state, pending approvals, and
artifact/materialization health. `fsck` verifies structural integrity including
raw artifact object identity and edges, resolution snapshots, envelopes,
construction evidence, and owned versus orphan materializations. Reopening Trail
recovers only staging owned by a provably dead exact process fence; unknown
materializations are reported for review instead of deleted automatically.

## Backups

Create, verify, and restore portable backup bundles:

```sh
trail backup create /tmp/trail-backup
trail backup verify /tmp/trail-backup
trail backup restore /tmp/trail-backup
```

Backups retain source uppers and recovery journals plus authoritative artifact
snapshots, objects, envelopes, attestations, historical generations, and exact
generation bindings. They omit mounted projections, generated/scratch uppers,
artifact materializations, and performance caches. Create, verify, and restore
reports expose retained private bytes and the count/known bytes of omitted state
as rebuildable. Verification seals the retained private tree by normalized path,
entry type, symlink target, and file content.

Restore rewrites lane workdir and retained-view paths so they point inside the
restored workspace, re-secures the private `.trail` and `.trail/index`
directories, retires copied active environment pointers, and rotates the
changed-path filesystem identity. The next environment sync reconstructs
materializations and caches from the retained authority. The next daemon-backed
command rebinds the observer to the restored host and reconciles the workspace
before trusting its incremental ledger again.

## Garbage Collection

Preview and run object pruning:

```sh
trail gc --dry-run
trail gc
```

Object GC now understands artifact CAS graphs. It retains content reachable
from generation bindings, layer shadows and pins, durable attempts and
resolution snapshots, attestations, quarantines, active holds, and recorded
materialization leases. It follows envelope, tree, directory, file, blob,
chunk-list, and chunk edges, so a chunk shared by several artifacts is removed
only after the last retained graph disappears.

Collection is deterministic and restartable: unreachable artifact DAGs are
ordered parent-before-child with object-ID tie breaking, and live deletion uses
256-object transactions. Every committed batch leaves the uncollected graph
valid, so an interrupted process can reopen and resume. Corrupt or ambiguous
reachability stops the operation without treating missing evidence as
permission to delete. Run
the cache collector before object GC when you also want an unused verified
materialization to stop retaining its reconstructible tree:

```sh
trail cache gc
trail gc
```

`trail lane space` and the structured `trail cache gc` report include an
`artifact_storage` object. It separates logical artifact content, authoritative
CAS bytes unique to one envelope, CAS bytes shared across envelopes, physical
materializations, lane-private allocation, demand-loaded projections,
persisted prefetch allocation, reclaimable bytes, and allocation that Trail
cannot safely attribute. These are multiple views of storage, not values to
sum: reclaimable bytes can also be materialized or demand-loaded, and logical
bytes are independent of both CAS encoding and filesystem allocation.

Trail deduplicates authoritative bytes by object ID and logical bytes by tree
root. Hot-set prefetch currently performs bounded reads into the operating
system page cache and therefore reports zero persisted prefetch bytes. Native
clone/reflink reports leave filesystem extents under `unknown_bytes` unless the
platform can prove their ownership; they do not invent shared savings.

Backup archives are self-contained and are created under the workspace write
lock; they do not pin the source workspace after publication.

## Code Facts Used

- Storage schema: `trail/src/db/storage/schema`
- Index rebuild/gc/backup: `trail/src/db/storage/lifecycle`, `trail/src/db/core/backup`
- Maintenance args: `trail/src/cli/command/maintenance_args.rs`
- Tests: `backup_create_verify_and_restore_roundtrip`, `index_rebuild_restores_derived_history_from_objects`, `gc_prunes_unreachable_known_objects_and_preserves_reachable_roots`
