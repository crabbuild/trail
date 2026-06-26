# Maintenance and Recovery

CrabDB includes local diagnostics, index rebuilds, backups, integrity checks, and object garbage collection.

## Doctor

```sh
crabdb doctor
```

Doctor reports operational health across workspace state and integrations. Tests verify it through the CLI, HTTP API, and MCP tool.

## Fsck

```sh
crabdb fsck
```

Use `fsck` to verify structural repository integrity.

## Index Rebuild

```sh
crabdb index rebuild
crabdb index rebuild --rich-text
```

Use rebuild when derived history/message indexes need to be reconstructed from stored objects.

## Worktree Index Watch

```sh
crabdb index watch --once
crabdb index watch --iterations 5 --interval-ms 1000
```

`--interval-ms` must be greater than zero. With `--format ndjson`, each watch iteration emits one JSON object per line.

## Backups

```sh
crabdb backup create /tmp/crabdb-backup
crabdb backup verify /tmp/crabdb-backup
crabdb backup restore /tmp/crabdb-backup
```

Use `--overwrite` when creating over an existing backup path and `--force` when restoring over an existing workspace.

## Garbage Collection

```sh
crabdb gc --dry-run
crabdb gc
```

Garbage collection prunes unreachable known objects while preserving reachable roots and referenced objects.

## Code Facts Used

- Maintenance CLI args: `crates/crabdb/src/cli/command/maintenance_args.rs`
- Maintenance handlers: `crates/crabdb/src/cli/command/handler/maintenance.rs`
- Tests: `doctor_reports_operational_health_across_cli_api_and_mcp`, `backup_create_verify_and_restore_roundtrip`, `gc_prunes_unreachable_known_objects_and_preserves_reachable_roots`

