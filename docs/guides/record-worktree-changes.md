# Record Worktree Changes

Use recording commands when you want Trail history to reflect the current workspace.

## Check Status

```sh
trail status
```

Status reports the current branch, head root, worktree state, and changed paths.

## Record All Visible Changes

```sh
trail record -m "describe the change"
```

If no files changed, the `operation` field in JSON output is `null`.

## Record Selected Paths

```sh
trail record --paths src README.md -m "record selected files"
```

Path selection can point at files or directories. Directory deletions are recorded as deleted files under that directory.

## Attach Operation Metadata

```sh
trail record \
  --kind manual-checkpoint \
  --session session-docs \
  -m "checkpoint ignored fixture"
```

Allowed `--kind` values are:

- `file-edit`
- `multi-file-edit`
- `format`
- `manual-checkpoint`
- `manual-record`

## Record Ignored Paths Deliberately

```sh
trail record --paths .env.local --allow-ignored -m "capture test fixture"
```

Only use this for intentional fixtures. Internal paths such as `.trail` remain protected.

## Watch and Record

```sh
trail watch --once --debounce-ms 100 --include-untracked -m "watched edit"
```

For a continuous loop, omit `--once` and tune `--interval-secs`.

## Code Facts Used

- CLI args: `trail/src/cli/command/worktree_args.rs`
- Record kind parsing: `trail/src/cli/command/handler/parsing.rs`
- Tests: `record_paths_records_only_selected_changes`, `record_kind_session_and_allow_ignored_path_are_audited`, `watch_cli_can_attach_recorded_operations_to_session`

