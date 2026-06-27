# First Lane Workflow

Lane workflows use a branch-backed work container, optional materialized
workdir, structured patches or recorded workdir edits, readiness checks, and a
merge path.

Use a lane for one active task. A lane is not an AI agent and does not launch
one by itself. It is the CrabDB work container that a human, script, editor, or
external coding agent can use.

## Daily Flow

```sh
cd /path/to/project

# Run once if this project is not initialized yet.
crabdb init --working-tree

crabdb lane spawn docs-lane --from main --materialize=true
crabdb lane status docs-lane
```

Open the lane workdir in your editor or point an external coding agent at it:

```sh
LANE_DIR="$(crabdb lane workdir docs-lane)"
cd "$LANE_DIR"
# Edit files or run a coding agent here.
```

Record the workdir changes back into the lane:

```sh
cd /path/to/project
crabdb lane record docs-lane -m "record docs update"
crabdb lane diff docs-lane --patch --show-line-ids
crabdb lane review docs-lane
crabdb lane readiness docs-lane
```

Run gates when the project requires them:

```sh
crabdb lane test docs-lane --suite unit -- cargo test -p crabdb
crabdb lane gates docs-lane --limit 20
```

Preview and queue the merge:

```sh
crabdb merge-lane docs-lane --into main --dry-run
crabdb merge-queue add docs-lane --into main
crabdb merge-queue run
```

Remove the lane after the work is merged or intentionally abandoned:

```sh
crabdb lane rm docs-lane --force
```

## Spawn Options

Create a virtual lane without a filesystem checkout:

```sh
crabdb lane spawn docs-lane --from main --no-materialize
```

Create a materialized workdir:

```sh
crabdb lane spawn docs-lane --from main --materialize=true
```

Materialize only selected paths:

```sh
crabdb lane spawn docs-lane --from main --materialize=true --paths docs README.md
```

## Structured Patch Flow

Create a patch JSON file:

```json
{
  "message": "add notes",
  "edits": [
    {
      "op": "write",
      "path": "docs/notes.md",
      "content": "notes\n"
    }
  ]
}
```

Apply it:

```sh
crabdb lane apply-patch docs-lane --patch patch.json
```

Structured patches are useful for MCP, ACP relay, editor, or script-driven
integrations because the lane branch can change without touching the main
workspace.

## Review the Lane

```sh
crabdb lane diff docs-lane --patch --show-line-ids
crabdb lane review docs-lane
crabdb lane status docs-lane
crabdb lane readiness docs-lane
crabdb lane contribution docs-lane
```

`readiness` reports blockers and warnings, including pending approvals, conflicts, dirty materialized workdirs, and required test/eval gates.

## Rewind a Lane

If a lane branch should return to a known-good state, rewind it and preserve
the failed head for review:

```sh
crabdb lane rewind docs-lane --to <change-or-root> --record-current --sync-workdir
```

Use `rewind` when an attempt goes sideways but the lane still contains useful
audit history.

## Merge

Preview first:

```sh
crabdb merge-lane docs-lane --into main --dry-run
```

For shared branches, use the queue:

```sh
crabdb merge-queue add docs-lane --into main
crabdb merge-queue run
```

If conflicts are opened:

```sh
crabdb conflicts list
crabdb conflicts show <conflict-set-id>
```

## Code Facts Used

- Lane CLI args: `crates/crabdb/src/cli/command/lane_args.rs`
- Merge queue args: `crates/crabdb/src/cli/command/collaboration_args/merge.rs`
- Patch schema: `crates/crabdb/src/model/inspect/patch.rs`
- Readiness: `crates/crabdb/src/db/lane/readiness.rs`
- Rewind: `crates/crabdb/src/db/lane/rewind.rs`
- Tests: `lane_patch_can_merge_into_main`, `merge_lane_and_queue_enforce_readiness_blockers`, `merge_queue_pauses_on_conflict`
