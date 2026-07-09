# Workspaces, Refs, and Branches

A Trail workspace is discovered from the current directory or selected with global flags and environment variables.

## Workspace Selection

Use:

```sh
trail --workspace /path/to/repo status
```

Or set:

```sh
export TRAIL_WORKSPACE=/path/to/repo
```

You can also point directly at a database directory:

```sh
trail --db /path/to/repo/.trail status
```

## Branch Selection

Use the global `--branch` option or `TRAIL_BRANCH` for commands that read the active branch.

```sh
trail --branch scratch status
```

Some commands also have command-specific branch flags, such as `status --branch`.

## Ref Names

The storage layer uses:

- `refs/branches/<name>` for normal branches.
- `refs/lanes/<name>` for lane branches.

Branches store the current `change_id`, `root_id`, generation, and update timestamp.

## Branch Commands

`trail branch` can:

- List branches when no action is supplied.
- Create a branch with `trail branch <name> --from <ref>`.
- Rename with `trail branch --rename <old> --to <new>`.
- Delete with `trail branch --delete <name>`.

## Code Facts Used

- Global args: `trail/src/cli/command.rs`
- Branch args: `trail/src/cli/command/worktree_args.rs`
- Ref storage: `trail/src/db/storage/refs.rs`
- Tests: `branch_list_rename_and_delete_work`, `cli_env_defaults_select_workspace_db_branch_and_format`

