# Initialize a Workspace

A Trail workspace is a normal directory with a `.trail` database directory. Initialize once per repository or worktree.

## Initialize an Empty Workspace

```sh
trail init
```

This creates the default branch state without importing current files.

## Initialize from the Current Working Tree

```sh
trail init --working-tree
```

This imports visible files from the working tree into the initial root.

## Initialize from Git-Tracked Files

```sh
trail init --from-git
```

This imports Git-tracked paths. The e2e suite verifies modified, deleted, and added tracked paths after this mode.

## Choose the Initial Branch

```sh
trail init --working-tree --branch main
```

The default branch is `main` when no `--branch` is supplied.

## Choose a Text Policy

```sh
trail init --working-tree --text-policy full
```

Supported policies are:

- `balanced`: default thresholds.
- `minimal`: favors lazy line tracking for large text and lower similarity preservation.
- `full`: favors full text maps and larger line limits.

The policy writes concrete config values such as `text.opaque_text_max_bytes`, `text.max_line_bytes`, and `text.preserve_similarity`.

## Files Created

Initialization creates Trail state under `.trail`, including SQLite metadata and
Prolly node storage under `.trail/index/trail.sqlite`, ref files, config, and
default ignore rules. It also creates `.trailignore` when needed.

Default `.trailignore` patterns include `.trail/`, `.git/`, `.env`, `.env.*`, private key file extensions, `node_modules/`, `target/`, `dist/`, `build/`, and `coverage/`.

## Code Facts Used

- Init args: `trail/src/cli/command/worktree_args.rs`
- Init behavior: `trail/src/db/core/init.rs`
- Text policy: `trail/src/db/util/config/policy.rs`
- Default ignore patterns: `trail/src/db/mod.rs`
- Tests: `init_record_why_and_fsck_work`, `init_text_policy_sets_text_tracking_thresholds`
