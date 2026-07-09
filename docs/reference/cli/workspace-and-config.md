# CLI Reference: Workspace and Config

## `init`

```text
trail init [--from-git] [--working-tree] [--branch <BRANCH>] [--text-policy <POLICY>] [--prolly-backend <BACKEND>] [--force]
```

Options:

- `--from-git`: import Git-tracked paths.
- `--working-tree`: import visible working tree files.
- `--branch <BRANCH>`: initial branch, default `main`.
- `--text-policy <minimal|balanced|full>`: configure text tracking thresholds.
- `--prolly-backend <sqlite|slatedb>`: choose the Prolly tree node backend for the new workspace.
- `--force`: allow initializing over existing state where the implementation permits it.

## `config`

```text
trail config list
trail config get <KEY>
trail config set <KEY> <VALUE>
```

Config values are typed and validated. See [Configuration reference](../config.md).
For production lane isolation and quota settings, see
[Hardening agent workflows](../../guides/hardening-agent-workflows.md).

## `ignore`

```text
trail ignore list
trail ignore add <PATTERN>
trail ignore remove <PATTERN>
trail ignore check <PATH>
```

Patterns are stored in `.trailignore`. Empty patterns, comment-only patterns, NULs, and line separators are rejected.

## `guardrails`

```text
trail guardrails check --action <ACTION> [--lane <LANE>] [--summary <TEXT>] [--payload-json <JSON>] [--path <PATH>...]
```

Returns `allowed`, `approval_required`, or `blocked`.

## Code Facts Used

- Args: `crates/trail/src/cli/command/workspace_args.rs`, `crates/trail/src/cli/command/worktree_args.rs`
- Config validation: `crates/trail/src/db/util/config`
- Ignore/guardrails: `crates/trail/src/db/core/workspace`
