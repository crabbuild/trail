# Initialize a Workspace

A CrabDB workspace is a normal directory with a `.crabdb` database directory. Initialize once per repository or worktree.

## Initialize an Empty Workspace

```sh
crabdb init
```

This creates the default branch state without importing current files.

## Initialize from the Current Working Tree

```sh
crabdb init --working-tree
```

This imports visible files from the working tree into the initial root.

## Initialize from Git-Tracked Files

```sh
crabdb init --from-git
```

This imports Git-tracked paths. The e2e suite verifies modified, deleted, and added tracked paths after this mode.

## Choose the Initial Branch

```sh
crabdb init --working-tree --branch main
```

The default branch is `main` when no `--branch` is supplied.

## Choose a Text Policy

```sh
crabdb init --working-tree --text-policy full
```

Supported policies are:

- `balanced`: default thresholds.
- `minimal`: favors lazy line tracking for large text and lower similarity preservation.
- `full`: favors full text maps and larger line limits.

The policy writes concrete config values such as `text.opaque_text_max_bytes`, `text.max_line_bytes`, and `text.preserve_similarity`.

## Files Created

Initialization creates CrabDB state under `.crabdb`, including SQLite storage under `.crabdb/index/crabdb.sqlite`, ref files, config, and default ignore rules. It also creates `.crabignore` when needed.

Default `.crabignore` patterns include `.crabdb/`, `.git/`, `.env`, `.env.*`, private key file extensions, `node_modules/`, `target/`, `dist/`, `build/`, and `coverage/`.

## Code Facts Used

- Init args: `crates/crabdb/src/cli/command/worktree_args.rs`
- Init behavior: `crates/crabdb/src/db/core/init.rs`
- Text policy: `crates/crabdb/src/db/util/config/policy.rs`
- Default ignore patterns: `crates/crabdb/src/db/mod.rs`
- Tests: `init_record_why_and_fsck_work`, `init_text_policy_sets_text_tracking_thresholds`

