# First Lane Workflow

Lane workflows use a branch-backed work container, optional materialized
workdir, structured patches or recorded workdir edits, readiness checks, and a
merge path.

Use a lane for one active task. A lane is not an AI agent and does not launch
one by itself. It is the Trail work container that a human, script, editor, or
external coding agent can use.

## Daily Flow

```sh
cd /path/to/project

# Run once if this project is not initialized yet.
trail init --working-tree

trail lane spawn docs-lane --from main
trail lane status docs-lane
```

Inspect the framework environment before launching tools. Discovery and planning
are side-effect free:

```sh
trail env discover docs-lane
trail env plan docs-lane
```

If discovery reports a component as `resolvable`, run the exact recovery command
from its report. To resolve every such component and prewarm the resulting
environment:

```sh
trail env resolve all docs-lane
trail env sync all docs-lane
trail env generation docs-lane
```

Resolution is explicit: managed execution never contacts a resolver on your
behalf. A current immutable snapshot is reused; `--refresh` deliberately reruns
the resolver. A project with no detected environment adapter may continue with
source-only lane work and should not run `env sync`.

Open the lane workdir in your editor or point an external coding agent at it:

```sh
LANE_DIR="$(trail lane workdir docs-lane)"
cd "$LANE_DIR"
# Edit files or run a coding agent here.
```

Record the workdir changes back into the lane:

```sh
cd /path/to/project
trail lane record docs-lane -m "record docs update"
trail lane diff docs-lane --patch --show-line-ids
trail lane review docs-lane
trail lane readiness docs-lane
```

Run gates when the project requires them:

```sh
trail lane test docs-lane --suite unit -- cargo test -p trail
trail lane gates docs-lane --limit 20
```

Preview and queue the merge:

```sh
trail lane merge docs-lane --into main --dry-run
trail lane merge-queue add docs-lane --into main
trail lane merge-queue run
```

Remove the lane after the work is merged or intentionally abandoned:

```sh
trail lane rm docs-lane --force
```

Removal retires the lane's environment generation and deletes its private and
disposable artifact state. Shared immutable CAS content remains only while it is
reachable or retained by cache policy.

## What Trail Reuses and What It Records

Trail records source and framework output through different pipelines:

| State | Identity and storage | Lane behavior | Git behavior |
| --- | --- | --- | --- |
| Source | Pinned source root plus Trail file/line identity | Copy-on-write source upper; checkpointed by `lane record` or managed finalization | Review, merge, then explicit Git handoff |
| Resolution snapshot | Immutable environment metadata | Reused when proposal and resolver pins match | Never source unless a declared source export says so |
| Shared artifact | Desired key → content root → verified artifact envelope | One immutable lower can serve sibling lanes | Not merged or committed |
| Private/disposable output | Lane, generation, component, and output binding | Fresh writable upper; copy-up and whiteouts stay private | Not merged or committed |
| Declared source export | Exact envelope/subtree, validation, destination, and authorization pins | Writes through normal source guardrails and checkpoints | Becomes an ordinary reviewable source change |

The desired key explains *why* an output may be reused. The content root says
*which bytes* were produced. The artifact envelope binds those identities to
validation, trust, output policy, and provenance. A filesystem materialization
is only a reconstructible projection of the authoritative CAS graph.

Use the artifact ID returned by structured environment reports to inspect or
verify that evidence:

```sh
trail --format json env generation docs-lane
trail env artifact inspect <ARTIFACT_ID>
trail env artifact verify <ARTIFACT_ID> --level full
```

Generated source is not a fifth output policy. A repository or adapter must
declare the export, then a user explicitly runs:

```sh
trail env source export docs-lane --component <COMPONENT> --export <NAME>
trail lane diff docs-lane --patch
```

## Spawn Options

Create a virtual lane without a filesystem checkout:

```sh
trail lane spawn docs-lane --from main --no-materialize
```

Create a materialized workdir:

```sh
trail lane spawn docs-lane --from main
```

Materialize only selected paths:

```sh
trail lane spawn docs-lane --from main --paths docs README.md
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
trail lane apply-patch docs-lane --patch patch.json
```

Structured patches are useful for MCP, ACP relay, editor, or script-driven
integrations because the lane branch can change without touching the main
workspace.

## Review the Lane

```sh
trail lane diff docs-lane --patch --show-line-ids
trail lane review docs-lane
trail lane status docs-lane
trail lane readiness docs-lane
trail lane contribution docs-lane
```

`readiness` reports blockers and warnings, including pending approvals, conflicts, dirty materialized workdirs, and required test/eval gates.

## Rewind a Lane

If a lane branch should return to a known-good state, rewind it and preserve
the failed head for review:

```sh
trail lane rewind docs-lane --to <change-or-root> --record-current --sync-workdir
```

Use `rewind` when an attempt goes sideways but the lane still contains useful
audit history.

## Merge

Preview first:

```sh
trail lane merge docs-lane --into main --dry-run
```

For shared branches, use the queue:

```sh
trail lane merge-queue add docs-lane --into main
trail lane merge-queue run
```

If conflicts are opened:

```sh
trail conflicts list
trail conflicts show <conflict-set-id>
```

## Code Facts Used

- Lane CLI args: `trail/src/cli/command/lane_args.rs`
- Merge queue args: `trail/src/cli/command/collaboration_args/merge.rs`
- Patch schema: `trail/src/model/inspect/patch.rs`
- Readiness: `trail/src/db/lane/readiness.rs`
- Rewind: `trail/src/db/lane/rewind.rs`
- Tests: `lane_patch_can_merge_into_main`, `merge_lane_and_queue_enforce_readiness_blockers`, `lane_merge_queue_pauses_on_conflict`
