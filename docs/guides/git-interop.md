# Git Interop

CrabDB can import from Git-tracked state, export patches, create Git commit objects, and list mappings between Git and CrabDB history.

## Initialize from Git

```sh
crabdb init --from-git
```

This imports Git-tracked paths into CrabDB.

## Import Current Git Snapshot

```sh
crabdb git import-update -m "sync git state"
```

This records the current Git-tracked snapshot on the selected CrabDB branch and stores mapping metadata.

## Export a Patch

Print a patch:

```sh
crabdb git export main..scratch
```

Write a patch file:

```sh
crabdb git export main..scratch --output change.patch
```

## Create a Git Commit Object

```sh
crabdb git export main..scratch -m "export CrabDB change"
```

`-m` cannot be combined with `--output`.

## Inspect Mappings

```sh
crabdb git mappings --limit 30
```

Mappings include direction, branch, Git head, whether Git was dirty, CrabDB change/root IDs, and timestamp.

## Code Facts Used

- Git CLI args: `crates/crabdb/src/cli/command/maintenance_args.rs`
- Git handlers: `crates/crabdb/src/cli/command/handler/maintenance.rs`
- Git storage: `crates/crabdb/src/db/storage/git.rs`
- Tests: `git_import_update_records_current_git_tracked_snapshot`, `git_export_with_message_creates_commit_object_and_mapping`

