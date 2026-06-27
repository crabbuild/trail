# Lane Overview

A CrabDB lane is a branch-backed work container. It has the normal code-state
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
crabdb lane spawn doc-bot --from main --materialize=true
crabdb lane workdir doc-bot
crabdb lane status doc-bot
crabdb lane diff doc-bot --patch
crabdb lane review doc-bot
crabdb lane readiness doc-bot
crabdb lane handoff doc-bot
crabdb lane rewind doc-bot --to <change-or-root> --record-current --sync-workdir
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
crabdb lane spawn docs-lane --from main --materialize=true
LANE_DIR="$(crabdb lane workdir docs-lane)"
cd "$LANE_DIR"
# Edit files or run an external coding agent here.

cd /path/to/project
crabdb lane record docs-lane -m "record task work"
crabdb lane diff docs-lane --patch
crabdb lane readiness docs-lane
crabdb merge-lane docs-lane --into main --dry-run
```

See [First lane workflow](../getting-started/first-lane-workflow.md) for the
full daily command sequence.

## Two Ways to Change a Lane Branch

Structured patches:

```sh
crabdb lane apply-patch doc-bot --patch patch.json
```

Materialized workdir recording:

```sh
crabdb lane workdir doc-bot
crabdb lane record doc-bot -m "record workdir edits"
```

## Review and Merge

Before merge, inspect contribution, readiness, gates, approvals, and diff.

```sh
crabdb lane contribution doc-bot
crabdb merge-lane doc-bot --into main --dry-run
```

If the branch should be abandoned back to a known-good state, use `lane rewind`
instead of silently moving refs. It records an auditable rewind operation and can
preserve the current head for later inspection.

## Code Facts Used

- Lane CLI surface: `crates/crabdb/src/cli/command/lane_args.rs`
- Lane models: `crates/crabdb/src/model/lane`
- Tests: `lane_management_commands_have_backing_apis`, `lane_patch_can_merge_into_main`
