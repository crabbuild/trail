# Git Interop

Trail can import from Git-tracked state, export patches, create Git commit objects, and list mappings between Git and Trail history.

## Initialize from Git

```sh
trail init --from-git
```

This imports Git-tracked paths into Trail.

## Import Current Git Snapshot

```sh
trail git import-update -m "sync git state"
```

This records the current Git-tracked snapshot on the selected Trail branch and stores mapping metadata.

## Export a Patch

Print a patch:

```sh
trail git export main..scratch
```

Write a patch file:

```sh
trail git export main..scratch --output change.patch
```

## Create a Git Commit Object

```sh
trail git export main..scratch -m "export Trail change"
```

`-m` cannot be combined with `--output`.

## Inspect Mappings

```sh
trail git mappings --limit 30
```

Mappings include direction, branch, Git head, whether Git was dirty, Trail change/root IDs, and timestamp.

## Code Facts Used

- Git CLI args: `trail/src/cli/command/maintenance_args.rs`
- Git handlers: `trail/src/cli/command/handler/maintenance.rs`
- Git storage: `trail/src/db/storage/git.rs`
- Tests: `git_import_update_records_current_git_tracked_snapshot`, `git_export_with_message_creates_commit_object_and_mapping`

