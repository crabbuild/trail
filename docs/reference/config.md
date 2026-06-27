# Configuration Reference

Use `crabdb config list`, `get`, and `set` to inspect and edit workspace config.

## Keys

| Key | Type | Read-only | Allowed values |
| --- | --- | --- | --- |
| `workspace.id` | string | yes | Generated workspace ID. |
| `workspace.default_branch` | string | no | Existing branch ref segment. |
| `recording.mode` | string | no | `save`, `manual`, `watch`. |
| `recording.debounce_ms` | u64 | no | Unsigned integer, zero allowed. |
| `recording.ignore_gitignored` | bool | no | Boolean parser values. |
| `text.small_text_max_bytes` | u64 | no | Greater than zero. |
| `text.tree_text_min_bytes` | u64 | no | Greater than zero. |
| `text.opaque_text_max_bytes` | u64 | no | Greater than zero. |
| `text.max_line_bytes` | u64 | no | Greater than zero. |
| `text.preserve_similarity` | f32 | no | Finite value from `0.0` to `1.0`. |
| `lane.default_materialize` | bool | no | Boolean parser values. |
| `lane.require_test_gate` | bool | no | Boolean parser values. |
| `lane.require_eval_gate` | bool | no | Boolean parser values. |
| `lane.required_test_suites` | list | no | Comma, semicolon, or newline separated suite names. |
| `lane.required_eval_suites` | list | no | Comma, semicolon, or newline separated suite names. |
| `lane.claim_enforcement` | string | no | `off`, `warn`, or `reject`. |
| `lane.enforce_sparse_paths` | bool | no | Boolean parser values. |
| `lane.max_patch_bytes` | u64 | no | Unsigned integer, zero disables the limit. |
| `lane.max_patch_file_bytes` | u64 | no | Unsigned integer, zero disables the limit. |
| `lane.max_changed_paths` | u64 | no | Unsigned integer, zero disables the limit. |
| `lane.max_event_payload_bytes` | u64 | no | Unsigned integer, zero disables the limit. |
| `lane.max_trace_payload_bytes` | u64 | no | Unsigned integer, zero disables the limit. |
| `lane.worktrees_dir` | path | no | Relative path normalized inside workspace. |
| `lane.merge_strategy` | string | no | `conservative`. |
| `git.export_trailers` | bool | no | Boolean parser values. |
| `guardrails.policy` | policy | no | `decision:scope:pattern` rules. |

## Boolean Parser

True values:

- `true`
- `1`
- `yes`
- `on`

False values:

- `false`
- `0`
- `no`
- `off`

## Text Policies at Init

`crabdb init --text-policy minimal|balanced|full` applies preset text thresholds before the config is written.

## Guardrail Policy Grammar

Rules are separated by semicolons or newlines:

```text
decision:scope:pattern
```

Decisions:

- `allow`
- `approval`
- `block`

Scopes:

- `action`
- `keyword`
- `path`

## Code Facts Used

- Config model: `crates/crabdb/src/model/domain/config.rs`
- Config entries: `crates/crabdb/src/db/util/config/entries.rs`
- Config validation: `crates/crabdb/src/db/util/config/set.rs`
- Parsing helpers: `crates/crabdb/src/db/util/config_parse.rs`
- Guardrail policy parser: `crates/crabdb/src/db/util/guardrails/policy.rs`
