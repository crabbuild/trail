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

## Choose a Prolly Storage Backend

```sh
trail init --working-tree --prolly-backend slatedb
```

The default backend is `sqlite`. `slatedb` stores Prolly tree nodes in SlateDB backed by the configured S3-compatible object store. The default local development settings use `http://localhost:9000`, bucket `crab`, and credentials `crab`/`crab`; inspect them with `trail config get storage.slatedb_s3_endpoint` or `trail config list`.

## Files Created

Initialization creates Trail state under `.trail`, including SQLite metadata storage under `.trail/index/trail.sqlite`, ref files, config, and default ignore rules. It also creates `.trailignore` when needed. With the SlateDB backend, Prolly tree nodes are stored outside SQLite under a workspace-scoped object-store path.

Default `.trailignore` patterns include `.trail/`, `.git/`, `.env`, `.env.*`, private key file extensions, `node_modules/`, `target/`, `dist/`, `build/`, and `coverage/`.

## Code Facts Used

- Init args: `crates/trail/src/cli/command/worktree_args.rs`
- Init behavior: `crates/trail/src/db/core/init.rs`
- Text policy: `crates/trail/src/db/util/config/policy.rs`
- Default ignore patterns: `crates/trail/src/db/mod.rs`
- Tests: `init_record_why_and_fsck_work`, `init_text_policy_sets_text_tracking_thresholds`
