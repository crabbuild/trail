# Use Case: Conflict Resolution

CrabDB records structured conflict sets when merges cannot be completed automatically.

## Open a Conflict Review

```sh
crabdb conflicts list
crabdb conflicts show <conflict-set-id>
```

Conflict show includes source and target refs, status, details, creation time, and an explanation packet. The explanation names the recorded merge changes, the conflicted paths, the source/target operations that touched them, best-effort logical line evidence when CrabDB can prove stable line identity, conservative resolution recommendations, and next steps.

## Resolve by Taking One Side

```sh
crabdb conflicts resolve <conflict-set-id> --take source
crabdb conflicts resolve <conflict-set-id> --take target
```

## Resolve Manually

Write a JSON map of paths to resolved text:

```json
{
  "README.md": "resolved content\n"
}
```

Or use the explicit `files` object:

```json
{
  "files": {
    "README.md": {
      "content": "resolved content\n",
      "executable": false
    },
    "old.md": {
      "delete": true
    }
  }
}
```

Then:

```sh
crabdb conflicts resolve <conflict-set-id> --manual resolution.json
```

## After Resolution

Run status, diff, and relevant test/eval gates:

```sh
crabdb status
crabdb diff --dirty --patch
crabdb agent readiness doc-bot
```

## Code Facts Used

- Conflict CLI args: `crates/crabdb/src/cli/command/collaboration_args/merge.rs`
- Manual resolution parsing: `crates/crabdb/src/cli/command/handler/parsing.rs`
- Tests: `merge_queue_pauses_on_conflict`, `manual_conflict_resolution_works_through_db_cli_http_and_mcp`
