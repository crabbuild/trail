# CLI Reference: Workspace and Config

## `init`

```text
crabdb init [--from-git] [--working-tree] [--branch <BRANCH>] [--text-policy <POLICY>] [--force]
```

Options:

- `--from-git`: import Git-tracked paths.
- `--working-tree`: import visible working tree files.
- `--branch <BRANCH>`: initial branch, default `main`.
- `--text-policy <minimal|balanced|full>`: configure text tracking thresholds.
- `--force`: allow initializing over existing state where the implementation permits it.

## `config`

```text
crabdb config list
crabdb config get <KEY>
crabdb config set <KEY> <VALUE>
```

Config values are typed and validated. See [Configuration reference](../config.md).

## `ignore`

```text
crabdb ignore list
crabdb ignore add <PATTERN>
crabdb ignore remove <PATTERN>
crabdb ignore check <PATH>
```

Patterns are stored in `.crabignore`. Empty patterns, comment-only patterns, NULs, and line separators are rejected.

## `guardrails`

```text
crabdb guardrails check --action <ACTION> [--agent <AGENT>] [--summary <TEXT>] [--payload-json <JSON>] [--path <PATH>...]
```

Returns `allowed`, `approval_required`, or `blocked`.

## Code Facts Used

- Args: `crates/crabdb/src/cli/command/workspace_args.rs`, `crates/crabdb/src/cli/command/worktree_args.rs`
- Config validation: `crates/crabdb/src/db/util/config`
- Ignore/guardrails: `crates/crabdb/src/db/core/workspace`

