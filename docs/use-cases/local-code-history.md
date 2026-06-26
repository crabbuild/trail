# Use Case: Local Code History

Use CrabDB to keep operation-level history for a local worktree, even before changes become Git commits.

## Flow

```sh
crabdb init --working-tree
crabdb status
crabdb record -m "start docs"
crabdb timeline --limit 10
crabdb diff <previous>..<current> --patch
```

## Why CrabDB Helps

- Records partial path selections without staging through Git.
- Stores operation messages and actor metadata.
- Preserves line identity across later provenance queries.
- Can export selected ranges to Git patches or commit objects.

## Good Fit

This is useful for iterative local work, editor automation, and change review before a Git commit boundary exists.

## Code Facts Used

- Worktree commands: `crates/crabdb/src/cli/command/worktree_args.rs`
- Git export: `crates/crabdb/src/db/merge/git_export.rs`
- Tests: `record_paths_records_only_selected_changes`, `git_export_with_message_creates_commit_object_and_mapping`

