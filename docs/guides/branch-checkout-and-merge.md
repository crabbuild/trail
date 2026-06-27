# Branch, Checkout, and Merge

CrabDB branches are local refs over operation roots.

## Create and Manage Branches

```sh
crabdb branch
crabdb branch scratch --from main
crabdb branch --rename scratch --to experiment
crabdb branch --delete experiment
```

Branch names are validated ref segments: they cannot be empty, contain `..`, start with `/`, contain backslashes, or contain NUL bytes.

## Checkout

```sh
crabdb checkout scratch --dry-run
crabdb checkout scratch
```

Safe options:

- `--dry-run`: report what would be written.
- `--workdir <PATH>`: materialize into an alternate empty or absent workdir.
- `--record-dirty`: record current workspace changes before materializing the target.
- `--force`: allow overwriting dirty workspace state when appropriate.

Checkout refuses unsafe materialization paths such as symlink workdirs.

## Merge

```sh
crabdb merge scratch --into main --dry-run
crabdb merge scratch --into main
```

Allowed strategies are `conservative`, `line-id-aware`, and `line_id_aware`.

When conflicts occur, CrabDB records structured conflict sets for inspection and resolution.

## Lane Merges

```sh
crabdb merge-lane doc-bot --into main --dry-run
crabdb merge-lane doc-bot --into main
```

Lane merges run readiness checks before mutating the target branch.

## Merge Queue

```sh
crabdb merge-queue add doc-bot --into main --priority 10
crabdb merge-queue list
crabdb merge-queue run --limit 1
crabdb merge-queue remove <queue-id>
```

The queue serializes merges and stops on conflicts or failures.

## Conflict Resolution

```sh
crabdb conflicts list
crabdb conflicts show <conflict-set-id>
crabdb conflicts resolve <conflict-set-id> --take source
crabdb conflicts resolve <conflict-set-id> --take target
crabdb conflicts resolve <conflict-set-id> --manual resolution.json
```

Manual JSON can be:

```json
{
  "README.md": "resolved content\n"
}
```

Or:

```json
{
  "files": {
    "README.md": {
      "content": "resolved content\n",
      "executable": false
    }
  }
}
```

## Code Facts Used

- Branch/checkout/merge args: `crates/crabdb/src/cli/command/worktree_args.rs`
- Merge/conflict args: `crates/crabdb/src/cli/command/collaboration_args/merge.rs`
- Conflict manual schema: `crates/crabdb/src/model/reports/merge.rs`
- Tests: `checkout_dry_run_and_alternate_workdir_are_safe`, `merge_dry_run_reports_conflicts_without_opening_conflict_state`, `manual_conflict_resolution_works_through_db_cli_http_and_mcp`

