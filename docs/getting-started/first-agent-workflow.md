# First Agent Workflow

Agent workflows use an agent branch, optional materialized workdir, structured patches or recorded workdir edits, readiness checks, and a merge path.

## Spawn an Agent Branch

```sh
crabdb agent spawn doc-bot --from main
```

To create a materialized workdir:

```sh
crabdb agent spawn doc-bot --from main --materialize=true
```

To materialize only selected paths:

```sh
crabdb agent spawn doc-bot --from main --materialize=true --paths docs README.md
```

## Apply a Structured Patch

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
crabdb agent apply-patch doc-bot --patch patch.json
```

## Review the Agent

```sh
crabdb agent diff doc-bot --patch --show-line-ids
crabdb agent review doc-bot
crabdb agent status doc-bot
crabdb agent readiness doc-bot
crabdb agent contribution doc-bot
```

`readiness` reports blockers and warnings, including pending approvals, conflicts, dirty materialized workdirs, and required test/eval gates.

## Merge

Preview first:

```sh
crabdb merge-agent doc-bot --into main --dry-run
```

For shared branches, use the queue:

```sh
crabdb merge-queue add doc-bot --into main
crabdb merge-queue run
```

If conflicts are opened:

```sh
crabdb conflicts list
crabdb conflicts show <conflict-set-id>
```

## Code Facts Used

- Agent CLI args: `crates/crabdb/src/cli/command/agent_args.rs`
- Merge queue args: `crates/crabdb/src/cli/command/collaboration_args/merge.rs`
- Patch schema: `crates/crabdb/src/model/inspect/patch.rs`
- Readiness: `crates/crabdb/src/db/agent/readiness.rs`
- Tests: `agent_patch_can_merge_into_main`, `merge_agent_and_queue_enforce_readiness_blockers`, `merge_queue_pauses_on_conflict`
