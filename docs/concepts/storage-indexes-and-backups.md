# Storage, Indexes, and Backups

CrabDB stores durable workspace state under `.crabdb` and maintains derived indexes for fast history and provenance queries.

## SQLite and Prolly Storage

The CrabDB index lives under:

```text
.crabdb/index/crabdb.sqlite
```

The `prolly` crate is re-exported from the `crabdb` crate and is used for map roots and content-addressed tree structures.

## Derived Indexes

Indexes support:

- File history.
- Line history.
- Message lookup.
- Session and agent operation lookup.
- Worktree file status acceleration.

Rebuild derived indexes with:

```sh
crabdb index rebuild
```

Use rich text hydration during rebuild when needed:

```sh
crabdb index rebuild --rich-text
```

Refresh the worktree file index:

```sh
crabdb index watch --once
```

## Health and Integrity

Use:

```sh
crabdb doctor
crabdb fsck
```

`doctor` checks operational readiness, schema version, current branch, `.crabignore` defaults, runtime integration state, and pending approvals. `fsck` verifies structural integrity.

## Backups

Create, verify, and restore portable backup bundles:

```sh
crabdb backup create /tmp/crabdb-backup
crabdb backup verify /tmp/crabdb-backup
crabdb backup restore /tmp/crabdb-backup
```

Restore rewrites materialized agent workdir paths so they point inside the restored workspace.

## Garbage Collection

Preview and run object pruning:

```sh
crabdb gc --dry-run
crabdb gc
```

## Code Facts Used

- Storage schema: `crates/crabdb/src/db/storage/schema`
- Index rebuild/gc/backup: `crates/crabdb/src/db/storage/lifecycle`, `crates/crabdb/src/db/core/backup`
- Maintenance args: `crates/crabdb/src/cli/command/maintenance_args.rs`
- Tests: `backup_create_verify_and_restore_roundtrip`, `index_rebuild_restores_derived_history_from_objects`, `gc_prunes_unreachable_known_objects_and_preserves_reachable_roots`

