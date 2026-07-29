# Lane Overview

A Trail lane is a branch-backed work container. It has the normal code-state
properties of a branch, plus the activity records needed to understand how the
work happened: sessions, turns, messages, events, spans, gates, approvals,
workdirs, and rewind checkpoints.

For a complete model of how lanes relate to branches, sessions, turns,
messages, gates, readiness, and merge flow, see [Lane work model](work-model.md).

Use a normal branch for long-lived code history such as `main` or `release`.
Use a lane for active work by a human, automation, or an external coding agent
such as Claude Code, Codex, or Cursor.

## Core Commands

```sh
trail lane spawn doc-bot --from main --materialize=true
trail lane workdir doc-bot
trail lane status doc-bot
trail lane diff doc-bot --patch
trail lane review doc-bot
trail lane readiness doc-bot
trail lane handoff doc-bot
trail lane rewind doc-bot --to <change-or-root> --record-current --sync-workdir
```

## Lane Branches

Each lane branch has:

- A base change and root.
- A head change and root.
- An optional current session.
- An optional materialized workdir.
- Provider and model metadata when supplied.

## Daily Workflow

Create one lane per task, use its workdir for normal tools, then record,
review, and merge the lane:

```sh
trail lane spawn docs-lane --from main --materialize=true
LANE_DIR="$(trail lane workdir docs-lane)"
cd "$LANE_DIR"
# Edit files or run an external coding agent here.

cd /path/to/project
trail lane record docs-lane -m "record task work"
trail lane diff docs-lane --patch
trail lane readiness docs-lane
trail lane merge docs-lane --into main --dry-run
```

See [First lane workflow](../getting-started/first-lane-workflow.md) for the
full daily command sequence.

## Two Ways to Change a Lane Branch

Structured patches:

```sh
trail lane apply-patch doc-bot --patch patch.json
```

Materialized workdir recording:

```sh
trail lane workdir doc-bot
trail lane record doc-bot -m "record workdir edits"
```

## Review and Merge

Before merge, inspect contribution, readiness, gates, approvals, and diff.

```sh
trail lane contribution doc-bot
trail lane merge doc-bot --into main --dry-run
```

If the branch should be abandoned back to a known-good state, use `lane rewind`
instead of silently moving refs. It records an auditable rewind operation and can
preserve the current head for later inspection.

## Archive, Remove, and Purge

These operations have deliberately different retention semantics:

```sh
trail lane archive doc-bot
trail lane unarchive doc-bot
trail lane rm doc-bot --force
trail lane purge <exact-lane-id> --force
```

- `archive` is reversible. It preserves lane identity, refs, workdir metadata,
  environment bindings, and provenance, but rejects new execution until the lane
  is unarchived.
- `rm` retires the lane. It checkpoints no new data, stops and removes Trail-owned
  runtime resources, unbinds environment generations and immutable layers,
  deletes the lane's private source/generated/scratch uppers, and retains a
  compact tombstone. Interrupted removal resumes from its durable phase when the
  workspace is reopened.
- `purge` irreversibly erases the completed tombstone and remaining lane-owned
  sessions, turns, messages, events, agent mappings, attestations, and provenance.
  It requires both an exact `lane_...` identity and `--force`.

Forking a layered lane with `--from <lane>` creates fresh private uppers and
reuses only verified, compatible immutable environment layers. Trail records
both inherited and rejected components in a `lane_environment_inheritance`
event.

## Managed Execution

`lane exec`, lane tests/evals, terminal agents, and materialized ACP prompt
turns use one lifecycle:

```text
resolve → discover/plan → sync-all → reconcile → mount → execute
        → checkpoint source → dispose runtime → unmount
```

Each phase emits a `managed_execution_phase` lane event. Command failure does
not skip source checkpointing or cleanup. Dependency, generated, scratch,
secret, and Trail-internal paths stay outside the recorded source change.

## Code Facts Used

- Lane CLI surface: `trail/src/cli/command/lane_args.rs`
- Lane models: `trail/src/model/lane`
- Tests: `lane_management_commands_have_backing_apis`, `lane_patch_can_merge_into_main`
