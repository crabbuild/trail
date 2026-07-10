# Safety and Recovery

Use Trail's safety signals as workflow inputs, not obstacles to bypass.

## Consequential Actions

Preview and obtain clear user intent before:

- Non-dry-run `trail agent apply` or `finish` because they can record a task workdir, create a Git commit, and fast-forward the current Git branch.
- Direct/shared-ref merge, `merge-queue run`, or conflict resolution.
- `undo`, `rewind`, checkout into an active workspace, forced workdir sync, lane removal, restore, non-dry-run GC, or destructive branch operations.
- Approval decisions, test/eval execution, network/deploy commands, or other open-world actions.

Do not use `--allow-stale`, `--allow-ignored`, `--force`, `--direct`, or daemon `--no-auth` unless the user explicitly accepts the concrete risk and the normal safe path cannot satisfy the request.

## Ignore, Secret, and Path Policy

Inspect rules before importing or recording uncertain paths:

```sh
trail ignore list
trail ignore check <path>
trail guardrails check --lane <lane> --action shell.exec --summary "<action>" --path <path>
```

Trail blocks internal/private paths, unsafe path forms, and secret-like structured patch content. Never weaken these protections to capture credentials, private keys, tokens, `.git`, or `.trail`. An ignored test fixture may be opted in only when its contents are reviewed and the user intends it to become Trail history.

Approval flow:

```sh
trail approvals request <lane> --action <action> --summary "<reason>"
trail approvals list --lane <lane> --status pending
trail approvals decide <approval-id> --decision approved --reviewer <name>
```

Do not self-approve on behalf of a human reviewer.

## Diagnose Before Recovery

For a high-level task:

```sh
trail agent diagnose <task>
trail agent delta <task> --patch
trail agent checkpoints <task>
```

For a lane:

```sh
trail lane status <lane>
trail lane timeline <lane>
trail lane diff <lane> --patch
trail lane readiness <lane>
```

Use `undo` for a prompt-sized agent turn. Use rewind for a known checkpoint/root and preserve the failed head:

```sh
trail lane rewind <lane> --to <change-or-root> --record-current --sync-workdir
```

Only sync a clean workdir. Verify the new delta immediately after recovery.

## Readiness and Conflicts

Readiness may block on dirty materialized workdirs, required or failed gates, pending approvals, open conflicts, invalid lanes, or policy failures. It may warn that a lane base is behind the target. Resolve the cause, then re-run readiness and dry-run.

For queued work:

```sh
trail merge-queue explain <lane>
trail lane refresh-preview <lane> --target main
```

For conflicts:

```sh
trail conflicts list
trail conflicts show <conflict-set-id>
```

Inspect stored base, target, and source evidence. Resolve each path deliberately; never choose ours/theirs solely to make the queue green. Re-run diff, gates, readiness, and merge dry-run after resolution.

## Error Handling

Use JSON for scripts:

```sh
trail --json <command>
```

Stable categories include `WORKSPACE_NOT_FOUND`, `INVALID_PATH`, `IGNORED_PATH`, `DIRTY_WORKTREE`, `MERGE_CONFLICT`, `PATCH_REJECTED`, `STALE_BRANCH`, `WORKSPACE_LOCKED`, `DATABASE_CORRUPT`, `GIT_ERROR`, and `DAEMON_UNAVAILABLE`.

Respond by category:

- Workspace missing: locate the intended root; initialize only if requested.
- Dirty worktree: inspect and preserve changes; never overwrite them.
- Patch rejected/stale: refresh lane head and regenerate the patch with a correct `base_change`.
- Merge conflict: inspect the conflict set and resolve with evidence.
- Locked/daemon unavailable: verify process/daemon health; do not delete lock or token files blindly.
- Database corrupt: stop writes, create a backup if possible, run `doctor`/`fsck`, and report recovery options.

## Maintenance Recovery

```sh
trail doctor
trail fsck
trail backup create /path/to/backup
trail backup verify /path/to/backup
trail index rebuild
trail gc --dry-run
```

Indexes are derived and rebuildable; object/ref corruption is different. Restore and GC only with explicit intent and after verifying a backup.
