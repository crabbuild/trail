# Maintenance and Recovery

Trail includes local diagnostics, index rebuilds, backups, integrity checks, and object garbage collection.

## Doctor

```sh
trail doctor
```

Doctor reports operational health across workspace state and integrations. Tests verify it through the CLI, HTTP API, and MCP tool.

Artifact diagnostics distinguish authoritative CAS corruption from a missing or
rebuildable materialization. Doctor reports active quarantine, incomplete/dead
construction owners, stale generation bindings, unknown materialization state,
and native backend prerequisites without treating an unavailable platform gate
as passed.

## Fsck

```sh
trail fsck
```

Use `fsck` to verify structural repository integrity.

Artifact fsck recomputes object identities and follows snapshot, envelope, tree,
directory, file, blob/chunk, validation, attestation, generation, quarantine,
hold, and materialization edges. Corrupt or ambiguous reachability fails closed;
it never becomes permission to collect bytes.

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

Backups retain authoritative artifact snapshots, CAS graphs, envelopes,
attestations, historical generations, bindings, source uppers, and exact-owner
recovery journals. They omit mounted projections, generated/scratch uppers,
verified materializations, and performance caches and report those omissions as
rebuildable. Restore retires copied active generation pointers and requires a
fresh `trail env sync all <LANE>` before execution.

## Garbage Collection

```sh
trail gc --dry-run
trail gc
```

Garbage collection prunes unreachable known objects while preserving reachable roots and referenced objects.

Run cache GC before object GC when reclaiming artifact projections:

```sh
trail cache gc --dry-run
trail cache gc
trail gc --dry-run
trail gc
```

Cache GC removes only unpinned reconstructible materializations. Object GC then
traces all live artifact authorities and deletes unreachable DAGs in restartable
bounded batches. Active generations, views, attempts, quarantines, holds,
attestations, and leases remain roots.

## Artifact Recovery

Start with read-only evidence:

```sh
trail doctor
trail fsck
trail --format json env status <LANE>
trail --format json env generation <LANE>
trail env artifact inspect <ARTIFACT_ID>
trail env artifact verify <ARTIFACT_ID> --level full
trail env artifact quarantine list
```

- For `resolvable`, run the exact `trail env resolve ...` recovery command in
  discovery/status output, then sync. Do not add a generated lock snapshot to
  source unless a declared source export authorizes it.
- For a missing or corrupt materialization with a valid envelope, rerun sync;
  Trail reconstructs it from CAS.
- For a dead exact-owner construction attempt, reopen the workspace so recovery
  fences and resumes or abandons that attempt before retrying sync.
- For quarantine, inspect both content identities and choose an explicit
  `retain-private`, `accept-incumbent`, `accept-candidate`, or `retire-all`
  resolution. Never delete rows or CAS files manually.
- After restore, rerun sync for each retained lane before managed execution.

## Interrupted Colima Guest Execution

`trail doctor` includes `managed_guest_executions` with bounded counts for
live, safely recoverable, ambiguous, and terminal receipts. Trail automatically
cleans an abandoned namespace only before candidate import. It fails closed for
an interrupted execute/export/import/checkpoint boundary because discarding or
reapplying state could overwrite lane work.

For a live command, use `trail lane exec-cancel <LANE>` or select its event
receipt with `--execution-id`. Cancellation is acknowledged only after the
owned guest process group has stopped and candidate import has been skipped.
If the owner process dies after the request, Trail validates the durable owner,
terminates the same guest process group, and cleans only that execution's
namespace.

For an ambiguous receipt, inspect the lane workdir and history, checkpoint
intentional source with `trail lane checkpoint <LANE>`, and retry after the
state is understood. Do not edit `.trail/managed-executions`, delete a guest
namespace, or stop the shared Colima profile as a repair shortcut.

## Schema-v1 Upgrade and Rollback Boundary

Trail still accepts exactly SQLite schema v1 and has no in-place database
migration path. The artifact pipeline extends the fresh schema-v1 contract; an
older schema-v1 workspace whose tables or validators no longer match fails
closed. Before installing a build with a changed storage contract:

1. Create and verify a backup with the old binary.
2. Preserve that backup and the old binary together.
3. Install the new binary and open a disposable restored copy first.
4. If Trail requests reinitialization, export accepted source to Git, preserve
   the backup, and use `trail init --force` rather than editing SQLite.

Rolling the binary back is safe only with a workspace/backup created under the
older binary's exact storage contract. A newer workspace is not downgraded in
place. Artifact CAS files, rows, refs, and generation bindings must never be
manually copied between the two states.

## Code Facts Used

- Maintenance CLI args: `trail/src/cli/command/maintenance_args.rs`
- Maintenance handlers: `trail/src/cli/command/handler/maintenance.rs`
- Tests: `doctor_reports_operational_health_across_cli_api_and_mcp`, `backup_create_verify_and_restore_roundtrip`, `gc_prunes_unreachable_known_objects_and_preserves_reachable_roots`
