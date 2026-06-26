# First Record and Query

This path records a normal file edit and asks CrabDB where a line came from.

## Start with a Workspace

```sh
crabdb init --working-tree
```

Edit a tracked text file, then inspect the dirty state:

```sh
crabdb status
```

## Record the Change

```sh
crabdb record -m "edit readme"
```

`record` scans the worktree, computes file and line changes, writes an operation, and advances the active branch.

To record only selected paths:

```sh
crabdb record --paths README.md -m "record readme only"
```

Directory selections are supported. The tests verify selected directory edits and selected directory deletions.

## Query Provenance

Ask why a line currently has its content:

```sh
crabdb why README.md:2
```

Show file or line history:

```sh
crabdb history README.md
```

List recent operations:

```sh
crabdb timeline --limit 10
```

Show a specific operation or object:

```sh
crabdb show <change-id>
```

## Inspect a Dirty Diff

```sh
crabdb diff --dirty --patch
```

Use `--show-line-ids` when an agent or reviewer needs stable line IDs for precise patching.

## Code Facts Used

- Record, status, diff args: `crates/crabdb/src/cli/command/worktree_args.rs`
- History/provenance args: `crates/crabdb/src/cli/command/inspect_args.rs`
- Record reports and operation model: `crates/crabdb/src/model/reports/worktree.rs`, `crates/crabdb/src/model/domain/operations.rs`
- Tests: `init_record_why_and_fsck_work`, `record_paths_records_only_selected_changes`, `diff_supports_roots_dirty_and_line_id_surfaces`

