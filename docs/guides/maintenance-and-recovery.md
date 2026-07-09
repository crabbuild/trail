# Maintenance and Recovery

Trail includes local diagnostics, index rebuilds, backups, integrity checks, and object garbage collection.

## Doctor

```sh
trail doctor
```

Doctor reports operational health across workspace state and integrations. Tests verify it through the CLI, HTTP API, and MCP tool.

## Fsck

```sh
trail fsck
```

Use `fsck` to verify structural repository integrity.

## Index Rebuild

```sh
trail index rebuild
trail index rebuild --rich-text
```

Use rebuild when derived history/message indexes need to be reconstructed from stored objects.

## Worktree Index Watch

```sh
trail index watch --once
trail index watch --iterations 5 --interval-ms 1000
```

`--interval-ms` must be greater than zero. With `--format ndjson`, each watch iteration emits one JSON object per line.

## Backups

```sh
trail backup create /tmp/trail-backup
trail backup verify /tmp/trail-backup
trail backup restore /tmp/trail-backup
```

Use `--overwrite` when creating over an existing backup path and `--force` when restoring over an existing workspace.

## Garbage Collection

```sh
trail gc --dry-run
trail gc
```

Garbage collection prunes unreachable known objects while preserving reachable roots and referenced objects.

## Code Facts Used

- Maintenance CLI args: `crates/trail/src/cli/command/maintenance_args.rs`
- Maintenance handlers: `crates/trail/src/cli/command/handler/maintenance.rs`
- Tests: `doctor_reports_operational_health_across_cli_api_and_mcp`, `backup_create_verify_and_restore_roundtrip`, `gc_prunes_unreachable_known_objects_and_preserves_reachable_roots`

