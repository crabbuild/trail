# Configure CrabDB

Use `crabdb config` to inspect and edit typed workspace config values.

## List Values

```sh
crabdb config list
```

## Read One Value

```sh
crabdb config get lane.default_materialize
```

## Set One Value

```sh
crabdb config set lane.require_test_gate true
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

Lane:

- `lane.default_materialize`
- `lane.require_test_gate`
- `lane.require_eval_gate`
- `lane.required_test_suites`
- `lane.required_eval_suites`
- `lane.claim_enforcement`: `off`, `warn`, or `reject`; controls whether active
  write claims/leases are enforced for lane patches and lane workdir records.
- `lane.enforce_sparse_paths`: turns sparse lane `--paths` selections into a
  hard write boundary.
- `lane.max_patch_bytes`: maximum serialized structured patch size; `0`
  disables the limit.
- `lane.max_patch_file_bytes`: maximum per-file patch or lane-record file size;
  `0` disables the limit.
- `lane.max_changed_paths`: maximum touched paths per lane mutation; `0`
  disables the limit.
- `lane.max_event_payload_bytes`: maximum lane event payload size; `0`
  disables the limit.
- `lane.max_trace_payload_bytes`: maximum trace span payload size; `0`
  disables the limit.
- `lane.worktrees_dir`
- `lane.merge_strategy`: currently `conservative`.

For agent-heavy workspaces, start with warning-mode hardening:

```sh
crabdb config set lane.claim_enforcement warn
crabdb config set lane.enforce_sparse_paths true
crabdb config set lane.max_changed_paths 25
crabdb config set lane.max_patch_bytes 1048576
crabdb config set lane.max_patch_file_bytes 262144
crabdb config set lane.max_event_payload_bytes 65536
crabdb config set lane.max_trace_payload_bytes 65536
```

After agents reliably claim or lease paths before writing, switch claims to
rejection:

```sh
crabdb config set lane.claim_enforcement reject
```

See [Hardening agent workflows](hardening-agent-workflows.md) for the full
operator model.

Git:

- `git.export_trailers`

Guardrails:

- `guardrails.policy`

## Boolean and List Values

Booleans accept `true`, `1`, `yes`, `on`, `false`, `0`, `no`, and `off`.

Suite lists can be comma, semicolon, or newline separated:

```sh
crabdb config set lane.required_test_suites "unit,integration"
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
