# Branch, Checkout, and Merge

Trail branches are local refs over operation roots.

## Create and Manage Branches

```sh
trail branch
trail branch scratch --from main
trail branch --rename scratch --to experiment
trail branch --delete experiment
```

Branch names are validated ref segments: they cannot be empty, contain `..`, start with `/`, contain backslashes, or contain NUL bytes.

## Checkout

```sh
trail checkout scratch --dry-run
trail checkout scratch
```

Safe options:

- `--dry-run`: report what would be written.
- `--workdir <PATH>`: materialize into an alternate empty or absent workdir.
- `--record-dirty`: record current workspace changes before materializing the target.
- `--force`: allow overwriting dirty workspace state when appropriate.

Checkout refuses unsafe materialization paths such as symlink workdirs.

## Merge

```sh
trail merge scratch --into main --dry-run
trail merge scratch --into main
```

Allowed strategies are `conservative`, `line-id-aware`, and `line_id_aware`.

When conflicts occur, Trail records structured conflict sets for inspection and resolution.

## Lane Merges

```sh
trail merge-lane doc-bot --into main --dry-run
```

Non-dry-run lane merges into the default branch use the merge queue by default:

```sh
trail merge-queue add doc-bot --into main
trail merge-queue run
```

Immediate default-branch merges require `trail merge-lane ... --direct`.

## Merge Queue

```sh
trail merge-queue add doc-bot --into main --priority 10
trail merge-queue list
trail merge-queue run --limit 1
trail merge-queue remove <queue-id>
```

The queue serializes merges and stops on conflicts or failures.

## Conflict Resolution

```sh
trail conflicts list
trail conflicts show <conflict-set-id>
trail conflicts resolve <conflict-set-id> --take source
trail conflicts resolve <conflict-set-id> --take target
trail conflicts resolve <conflict-set-id> --manual resolution.json
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

Manual resolution objects accept only `content`, `delete`, and `executable`;
unknown keys are rejected.

## Code Facts Used

- Branch/checkout/merge args: `crates/trail/src/cli/command/worktree_args.rs`
- Merge/conflict args: `crates/trail/src/cli/command/collaboration_args/merge.rs`
- Conflict manual schema: `crates/trail/src/model/reports/merge.rs`
- Tests: `checkout_dry_run_and_alternate_workdir_are_safe`, `merge_dry_run_reports_conflicts_without_opening_conflict_state`, `manual_conflict_resolution_works_through_db_cli_http_and_mcp`
