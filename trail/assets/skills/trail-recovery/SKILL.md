---
name: trail-recovery
description: Diagnose and recover blocked or unhealthy Trail workspaces, lanes, and managed agent tasks. Use when Trail reports dirty work, stale state, rejected patches, failed readiness or gates, conflicts, locks, daemon failure, schema incompatibility, database corruption, interrupted lane lifecycle, or backup/restore concerns; or when a user asks to undo, rewind, repair, fsck, rebuild indexes, garbage-collect, restore, or remove Trail state safely.
---

# Trail Recovery

Treat Trail's blockers as evidence. Diagnose first, preserve user work and durable history, then choose the narrowest recovery that addresses the proven cause.

## Start Read-Only

```sh
trail --format json status
trail doctor
trail fsck
```

For a managed task, start with `trail agent diagnose <task>`, `delta --patch`, and `checkpoints`. For a lane, start with `lane status`, `timeline`, `diff --patch`, and `readiness`.

Read [recovery-playbook.md](references/recovery-playbook.md) before any rewind, conflict resolution, forced sync, restore, non-dry-run garbage collection, lane removal, or schema recovery.

## Never Bypass the Cause

Do not reach for `--force`, `--allow-stale`, `--allow-ignored`, `--direct`, or `--no-auth` merely to make a command succeed. Do not delete locks, database files, refs, tokens, journals, quarantine paths, or `.trail` internals manually.

Back up before invasive recovery. Preview when available. After recovery, rerun the original diagnostic plus status/diff/readiness and report what evidence changed, what remains blocked, and whether any gate was skipped.
