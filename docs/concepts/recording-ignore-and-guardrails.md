# Recording, Ignore Rules, and Guardrails

CrabDB records visible workspace changes and protects ignored or private paths from accidental capture.

## Recording

`crabdb record` creates a new operation when there are changed paths. It supports:

- `-m, --message`
- `--paths <PATH>...`
- `--kind`
- `--session`
- `--allow-ignored`

`crabdb watch` polls on an interval and records detected changes. It supports `--once`, `--interval-secs`, `--debounce-ms`, `--include-untracked`, and optional session linkage.

## Ignore Rules

CrabDB creates default `.crabignore` rules for private, generated, and large dependency/build paths. It also has hardcoded private path protections. Hardcoded ignored paths return source `hardcoded`; workspace and `.gitignore` matches return source `workspace`.

Manage rules with:

```sh
crabdb ignore list
crabdb ignore add notes.secret
crabdb ignore check notes.secret
crabdb ignore remove notes.secret
```

Lane patches and selected records reject ignored paths unless the operation explicitly opts in with `allow_ignored` or `--allow-ignored`. Internal paths such as `.crabdb` remain blocked.

## Guardrails

Guardrails preflight proposed agent actions:

```sh
crabdb guardrails check --action shell.exec --summary "run tests" --path README.md
```

Decisions are:

- `allowed`
- `approval_required`
- `blocked`

Classification marks shell/process execution, network access, deploy/release/publish work, destructive changes, and policy edits as approval-required. Destructive host-level commands and hardcoded private paths are blocked.

Workspace policy rules use:

```text
decision:scope:pattern
```

Where decision is `allow`, `approval`, or `block`, and scope is `action`, `keyword`, or `path`.

## Code Facts Used

- Record/watch args: `crates/crabdb/src/cli/command/worktree_args.rs`
- Ignore behavior: `crates/crabdb/src/db/core/workspace/ignore.rs`, `crates/crabdb/src/db/util/path.rs`
- Guardrail behavior: `crates/crabdb/src/db/core/workspace/guardrails.rs`, `crates/crabdb/src/db/util/guardrails`
- Tests: `ignore_cli_manages_crabignore_and_status`, `lane_patch_respects_ignore_policy_and_explicit_opt_in`, `local_api_and_mcp_expose_ignore_controls`

