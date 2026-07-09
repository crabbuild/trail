# Storage, Indexes, and Backups

Trail stores durable workspace state under `.trail` and maintains derived indexes for fast history and provenance queries.

## SQLite and Prolly Storage

The Trail index lives under:

```text
.trail/index/trail.sqlite
```

The `prolly` crate is re-exported from the `trail` crate and is used for map roots and content-addressed tree structures.

By default, Prolly tree nodes are stored in SQLite. New workspaces can opt into SlateDB-backed node storage with:

```sh
trail init --working-tree --prolly-backend slatedb
```

SlateDB uses the `storage.slatedb_*` config keys and writes nodes to the configured S3-compatible object store. The SQLite database remains the local metadata store for refs, operations, derived indexes, and workspace bookkeeping.

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

`doctor` checks operational readiness, schema version, current branch, `.trailignore` defaults, runtime integration state, and pending approvals. `fsck` verifies structural integrity.

## Backups

Create, verify, and restore portable backup bundles:

```sh
trail backup create /tmp/trail-backup
trail backup verify /tmp/trail-backup
trail backup restore /tmp/trail-backup
```

Restore rewrites materialized lane workdir paths so they point inside the restored workspace.

## Garbage Collection

Preview and run object pruning:

```sh
trail gc --dry-run
trail gc
```

## Code Facts Used

- Storage schema: `trail/src/db/storage/schema`
- Index rebuild/gc/backup: `trail/src/db/storage/lifecycle`, `trail/src/db/core/backup`
- Maintenance args: `trail/src/cli/command/maintenance_args.rs`
- Tests: `backup_create_verify_and_restore_roundtrip`, `index_rebuild_restores_derived_history_from_objects`, `gc_prunes_unreachable_known_objects_and_preserves_reachable_roots`
