# Configuration Reference

Use `trail config list`, `get`, and `set` to inspect and edit workspace config.

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
| `storage.prolly_backend` | string | yes | `sqlite` or `slatedb`. |
| `storage.slatedb_path` | string | no | Non-empty object-store path. |
| `storage.slatedb_s3_endpoint` | string | no | Non-empty S3-compatible endpoint URL. |
| `storage.slatedb_s3_bucket` | string | no | Non-empty bucket name. |
| `storage.slatedb_s3_region` | string | no | Non-empty region. |
| `storage.slatedb_s3_access_key_id` | string | no | Non-empty access key ID. |
| `storage.slatedb_s3_secret_access_key` | string | no | Non-empty secret access key. |
| `storage.slatedb_s3_allow_http` | bool | no | Boolean parser values. |
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

`trail init --text-policy minimal|balanced|full` applies preset text thresholds before the config is written.

## Prolly Storage Backend

`trail init --prolly-backend sqlite|slatedb` chooses where Prolly tree nodes are stored for a new workspace. `sqlite` keeps nodes in `.trail/index/trail.sqlite`. `slatedb` stores nodes in SlateDB backed by the configured S3-compatible object store.

When initialized with `--prolly-backend slatedb`, Trail writes a workspace-scoped default `storage.slatedb_path` of `trail/workspaces/<workspace-id>/prolly`. The default S3-compatible settings target local RustFS-compatible development:

```text
storage.slatedb_s3_endpoint = "http://localhost:9000"
storage.slatedb_s3_bucket = "crab"
storage.slatedb_s3_region = "us-east-1"
storage.slatedb_s3_access_key_id = "crab"
storage.slatedb_s3_secret_access_key = "crab"
storage.slatedb_s3_allow_http = true
```

## Lane Hardening Keys

`lane.claim_enforcement` controls whether active write claims/leases are treated
as a policy boundary for lane patches and materialized workdir records:

- `off`: claims and leases remain advisory only.
- `warn`: mutations outside active write claims/leases are allowed, but Trail
  records a `lane_policy_warning` event.
- `reject`: mutations outside active write claims/leases are rejected.

Read leases do not grant write permission. Use `warn` first when introducing
the policy to an existing workspace, then switch to `reject` after agents
consistently claim their intended paths.

`lane.enforce_sparse_paths` turns sparse lane `--paths` selections into a hard
write boundary. When true, lane patches and materialized workdir records must
stay inside the persisted sparse selection. Rename source and destination paths
are both checked. The sparse selection is stored in lane metadata so policy
enforcement still works if the workdir sparse manifest is missing.

The lane quota keys all accept unsigned integers. A value of `0` disables the
limit.

| Key | Enforced on |
| --- | --- |
| `lane.max_patch_bytes` | Serialized structured patch document before storage. |
| `lane.max_patch_file_bytes` | Per-file structured patch writes and materialized lane-record files. |
| `lane.max_changed_paths` | Number of paths touched by a lane patch or lane workdir record. |
| `lane.max_event_payload_bytes` | Lane event payload JSON before storage, before and after redaction. |
| `lane.max_trace_payload_bytes` | `span_started` and `span_ended` payload JSON before trace indexing. |

Example hardened profile:

```sh
trail config set lane.claim_enforcement warn
trail config set lane.enforce_sparse_paths true
trail config set lane.max_changed_paths 25
trail config set lane.max_patch_bytes 1048576
trail config set lane.max_patch_file_bytes 262144
trail config set lane.max_event_payload_bytes 65536
trail config set lane.max_trace_payload_bytes 65536
```

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

- Config model: `crates/trail/src/model/domain/config.rs`
- Config entries: `crates/trail/src/db/util/config/entries.rs`
- Config validation: `crates/trail/src/db/util/config/set.rs`
- Parsing helpers: `crates/trail/src/db/util/config_parse.rs`
- Guardrail policy parser: `crates/trail/src/db/util/guardrails/policy.rs`
