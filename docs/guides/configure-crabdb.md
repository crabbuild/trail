# Configure CrabDB

Use `crabdb config` to inspect and edit typed workspace config values.

## List Values

```sh
crabdb config list
```

## Read One Value

```sh
crabdb config get agent.default_materialize
```

## Set One Value

```sh
crabdb config set agent.require_test_gate true
```

## Important Groups

Workspace:

- `workspace.id`: read-only workspace ID.
- `workspace.default_branch`: must name an existing branch.

Recording:

- `recording.mode`: `save`, `manual`, or `watch`.
- `recording.debounce_ms`: unsigned integer.
- `recording.ignore_gitignored`: boolean.

Text:

- `text.small_text_max_bytes`
- `text.tree_text_min_bytes`
- `text.opaque_text_max_bytes`
- `text.max_line_bytes`
- `text.preserve_similarity`: float from `0.0` to `1.0`.

Agent:

- `agent.default_materialize`
- `agent.require_test_gate`
- `agent.require_eval_gate`
- `agent.required_test_suites`
- `agent.required_eval_suites`
- `agent.worktrees_dir`
- `agent.merge_strategy`: currently `conservative`.

Git:

- `git.export_trailers`

Guardrails:

- `guardrails.policy`

## Boolean and List Values

Booleans accept `true`, `1`, `yes`, `on`, `false`, `0`, `no`, and `off`.

Suite lists can be comma, semicolon, or newline separated:

```sh
crabdb config set agent.required_test_suites "unit,integration"
```

## Guardrail Policy Values

Policy rules use:

```text
decision:scope:pattern
```

Example:

```sh
crabdb config set guardrails.policy "block:path:production;approval:keyword:deploy"
```

## Code Facts Used

- Config command args: `crates/crabdb/src/cli/command/workspace_args.rs`
- Config entries and validation: `crates/crabdb/src/db/util/config`
- Tests: `config_api_lists_sets_persists_and_validates_keys`, `required_gate_config_blocks_merge_until_test_and_eval_pass`

