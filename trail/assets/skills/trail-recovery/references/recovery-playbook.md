# Recovery Playbook

## Classify the Failure

- Dirty worktree: inspect and record, rescue, or deliberately discard only the exact user-authorized changes.
- Rejected or stale patch: refresh the lane head and regenerate the patch with the current `base_change`, line identity, and expected text.
- Readiness failure: resolve dirty work, failed/missing gates, approvals, stale environments, conflicts, or policy errors; then rerun readiness and the merge/apply dry-run.
- Conflict: inspect the stored base, target, and source evidence. Never choose ours/theirs only to make a queue green.
- Workspace locked or daemon unavailable: verify process and daemon health. Do not remove lock or token files blindly.
- Database or object corruption: stop mutations, create and verify a backup if possible, then use doctor/fsck evidence to choose repair or reinitialization.
- Schema incompatibility: Trail has no database migration path. Preserve a backup and follow the exact `trail init --force --from-git` guidance.

## Preserve History During Rewind

For a managed task:

```sh
trail agent diagnose <task>
trail agent delta <task> --patch
trail agent checkpoints <task>
```

Use task `undo` for one prompt-sized turn and `rewind --to` for a known checkpoint. For a direct lane:

```sh
trail lane rewind <lane> --to <change-or-root> --record-current --sync-workdir
```

Only synchronize a clean workdir. Preserve the failed head and verify the resulting diff immediately.

## Maintenance and Backup

```sh
trail backup create /path/to/backup
trail backup verify /path/to/backup
trail gc --dry-run
```

Derived indexes may be rebuilt when diagnostics prove they are the problem. Object/ref corruption is different and must not be hidden by an index rebuild. Restore, non-dry-run GC, destructive branch/lane removal, and force/overwrite options require explicit intent and a verified backup.
