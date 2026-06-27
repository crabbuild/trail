# Workspaces, Refs, and Branches

A CrabDB workspace is discovered from the current directory or selected with global flags and environment variables.

## Workspace Selection

Use:

```sh
crabdb --workspace /path/to/repo status
```

Or set:

```sh
export CRABDB_WORKSPACE=/path/to/repo
```

You can also point directly at a database directory:

```sh
crabdb --db /path/to/repo/.crabdb status
```

## Branch Selection

Use the global `--branch` option or `CRABDB_BRANCH` for commands that read the active branch.

```sh
crabdb --branch scratch status
```

Some commands also have command-specific branch flags, such as `status --branch`.

## Ref Names

The storage layer uses:

- `refs/branches/<name>` for normal branches.
- `refs/lanes/<name>` for lane branches.

Branches store the current `change_id`, `root_id`, generation, and update timestamp.

## Branch Commands

`crabdb branch` can:

- List branches when no action is supplied.
- Create a branch with `crabdb branch <name> --from <ref>`.
- Rename with `crabdb branch --rename <old> --to <new>`.
- Delete with `crabdb branch --delete <name>`.

## Code Facts Used

- Global args: `crates/crabdb/src/cli/command.rs`
- Branch args: `crates/crabdb/src/cli/command/worktree_args.rs`
- Ref storage: `crates/crabdb/src/db/storage/refs.rs`
- Tests: `branch_list_rename_and_delete_work`, `cli_env_defaults_select_workspace_db_branch_and_format`

