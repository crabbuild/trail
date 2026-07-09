# Use Case: Local Code History

Use Trail to keep operation-level history for a local worktree, even before changes become Git commits.

## Flow

```sh
trail init --working-tree
trail status
trail record -m "start docs"
trail timeline --limit 10
trail diff <previous>..<current> --patch
```

## Why Trail Helps

- Records partial path selections without staging through Git.
- Stores operation messages and actor metadata.
- Preserves line identity across later provenance queries.
- Can export selected ranges to Git patches or commit objects.

## Good Fit

This is useful for iterative local work, editor automation, and change review before a Git commit boundary exists.

## Code Facts Used

- Worktree commands: `crates/trail/src/cli/command/worktree_args.rs`
- Git export: `crates/trail/src/db/merge/git_export.rs`
- Tests: `record_paths_records_only_selected_changes`, `git_export_with_message_creates_commit_object_and_mapping`

