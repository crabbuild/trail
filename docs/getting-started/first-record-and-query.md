# First Record and Query

This path records a normal file edit and asks Trail where a line came from.

## Start with a Workspace

```sh
trail init --working-tree
```

Edit a tracked text file, then inspect the dirty state:

```sh
trail status
```

## Record the Change

```sh
trail record -m "edit readme"
```

`record` scans the worktree, computes file and line changes, writes an operation, and advances the active branch.

To record only selected paths:

```sh
trail record --paths README.md -m "record readme only"
```

Directory selections are supported. The tests verify selected directory edits and selected directory deletions.

## Query Provenance

Ask why a line currently has its content:

```sh
trail why README.md:2
```

Show file or line history:

```sh
trail history README.md
```

List recent operations:

```sh
trail timeline --limit 10
```

Show a specific operation or object:

```sh
trail show <change-id>
```

## Inspect a Dirty Diff

```sh
trail diff --dirty --patch
```

Use `--show-line-ids` when an agent or reviewer needs stable line IDs for precise patching.

## Code Facts Used

- Record, status, diff args: `trail/src/cli/command/worktree_args.rs`
- History/provenance args: `trail/src/cli/command/inspect_args.rs`
- Record reports and operation model: `trail/src/model/reports/worktree.rs`, `trail/src/model/domain/operations.rs`
- Tests: `init_record_why_and_fsck_work`, `record_paths_records_only_selected_changes`, `diff_supports_roots_dirty_and_line_id_surfaces`

