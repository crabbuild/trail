# Hardening Agent Workflows

This guide collects the production hardening controls for running humans,
automation, and coding agents against the same Trail workspace. The defaults
stay compatible with lightweight local use, while config flags can turn
coordination hints into enforceable boundaries.

## Recommended Hardened Profile

Start with warnings, then move to rejection after existing workflows are clean.

```sh
trail config set lane.claim_enforcement warn
trail config set lane.enforce_sparse_paths true
trail config set lane.max_changed_paths 25
trail config set lane.max_patch_bytes 1048576
trail config set lane.max_patch_file_bytes 262144
trail config set lane.max_event_payload_bytes 65536
trail config set lane.max_trace_payload_bytes 65536
```

After agents consistently claim or lease their intended paths:

```sh
trail config set lane.claim_enforcement reject
```

Use merge queue for shared targets instead of direct merges:

```sh
trail lane merge-queue add docs-lane --into main
trail lane merge-queue explain docs-lane
trail lane merge-queue run
```

## Lane Isolation

Path claims and leases are advisory by default. With
`lane.claim_enforcement=warn`, Trail records a `lane_policy_warning` event when
a lane patch or materialized workdir record touches a path outside active write
claims/leases. With `reject`, the same mutation is blocked. Read leases do not
grant write permission.

```sh
trail lane claim docs-lane docs README.md --ttl-secs 1800
trail lane record docs-lane --preview
```

Sparse materialization can also become a hard boundary. When a lane is spawned
with selected paths and `lane.enforce_sparse_paths=true`, lane patches and
workdir records must stay inside those selected paths. Rename source and
destination paths are checked. Trail persists the sparse boundary in lane
metadata, so enforcement survives a missing `.trail/sparse-workdir.json` and
can recreate the manifest after a valid sparse update.

```sh
trail lane spawn docs-lane --from main --materialize=true --paths docs README.md
trail config set lane.enforce_sparse_paths true
```

Quotas provide blast-radius limits:

- `lane.max_patch_bytes`: serialized structured patch size.
- `lane.max_patch_file_bytes`: per-file patch/write or lane-record file size.
- `lane.max_changed_paths`: maximum touched paths per lane mutation.
- `lane.max_event_payload_bytes`: lane event payload size before storage.
- `lane.max_trace_payload_bytes`: trace span payload size before indexing.

Zero disables each quota.

## Patch And Path Safety

Structured lane patches should carry the current lane head in `base_change`.
Direct lane patches reject stale or missing bases unless `allow_stale=true` is
set deliberately. Turn-linked patches may omit `base_change` because the turn's
`before_change` is used as the freshness guard.

```json
{
  "base_change": "current-lane-head-change-id",
  "message": "safe edit",
  "edits": [
    { "op": "write", "path": "docs/notes.md", "content": "text\n" }
  ]
}
```

For sensitive text edits, prefer `replace_line` with both `line_id` and
`expected_text`. Trail rejects missing or mismatched expected text before
mutating the lane.

```json
{
  "base_change": "current-lane-head-change-id",
  "edits": [
    {
      "op": "replace_line",
      "path": "README.md",
      "line_id": "line_abc:2",
      "expected_text": "old text",
      "new_text": "new text"
    }
  ]
}
```

Patch paths are normalized before use. Trail rejects parent-directory escapes,
absolute paths, backslash separators on non-Windows external paths, non-NFC
Unicode, invisible Unicode format controls, separator lookalikes,
case-insensitive collisions, Windows reserved names/aliases, `.trail`, `.git`,
hardcoded private paths, and ignored paths unless explicitly allowed.

Patch messages and edit payloads are secret-scanned before storage. Event and
trace payloads are checked against size limits before and after redaction so a
large secret-bearing payload cannot bypass quotas by shrinking after redaction.

## Materialized Workdirs

Before recording a materialized lane workdir, preview it:

```sh
trail lane record docs-lane --preview --json
```

The preview reports:

- `changed_paths`: additions, modifications, deletes, renames, and mode changes.
- `ignored_paths`: paths matched by `.trailignore` or `.gitignore`.
- `risky_paths`: nested `.git`, nested `.trail`, symlinks, hardlinks, and
  best-effort external mount/device boundaries.
- `oversized_files`: changed files over `lane.max_patch_file_bytes`.
- `policy`: whether the current record would pass path and quota policy.

Workdir sync and force refresh are transactional: Trail stages the new workdir
contents, verifies the manifest, then swaps or updates. If force refresh must
replace an existing non-directory or symlink workdir path, it writes a rescue
copy beside the workdir before promotion.

Dirty workdir detection includes content, deletions, renames, mode and
executable-bit changes, sparse hydration, ignored-file edge cases, and manifest
fallback behavior.

## Merge Safety

For shared targets such as `main`, use merge queue as the default. Non-dry-run
direct merges into shared targets require explicit `--direct`.

Queue execution re-runs readiness immediately before each queued item merges.
Readiness blocks dirty workdirs, missing or failed required gates, pending
approvals, open conflicts, invalid/removed lanes, and configured gate failures.
Readiness also warns when the lane base is behind the default branch, for
example `lane started 14 operations behind main`.

Before merging, inspect the queue item:

```sh
trail lane merge-queue explain docs-lane
```

`merge-queue explain` includes readiness blockers, dry-run conflicts, preflight
errors, warnings, and next-step commands. Conflict records store base, target,
and source root snapshots so show/resolve remains reproducible even if refs move
later. Conflict explanations classify paths as `modify/modify`,
`delete/modify`, `rename/modify`, `binary`, `mode`, or
`same_insertion_gap`, and repeated conflicts can surface known-resolution
suggestions from earlier resolutions.

Use refresh preview to understand a stale lane before merging:

```sh
trail lane refresh-preview docs-lane --target main
```

It reports operations behind, incoming target changes, conflicts, changed paths,
and next steps without mutating refs or recording conflict state.

## Daemon, HTTP, And MCP

The daemon defaults to authenticated loopback operation. `--no-auth` is allowed
only on loopback listeners, cannot be combined with token flags, and prints a
stderr warning. Token files must be regular files; symlink token files are
rejected. On Unix, generated or reused token files are restricted to `0600`.

HTTP hardening includes:

- 16 MiB total request limit.
- Connection read/write timeouts.
- Per-peer listener rate limits.
- Strict request-line and header parsing.
- Duplicate sensitive header rejection.
- Required loopback `Host` for routed requests.
- Loopback-only `Origin` when `Origin` is present.
- Strict JSON request objects and strict OpenAPI schemas.
- Non-empty body rejection for bodyless mutation routes.
- Optional `Idempotency-Key` replay for mutating requests.
- Durable audit rows for non-GET mutation attempts, including unauthorized and
  forbidden attempts.

MCP hardening includes strict JSON-RPC envelopes, strict params and tool
arguments, reserved `_meta` support, read-only tool enforcement through a
read-only database/sidecar guard, and external mutation audit records for
mutating `tools/call` attempts.

Audit rows are stored in `external_mutation_audit` with actor, surface, command,
target ref, lane/turn context when known, status/result, change ID when
available, and a compact redacted summary. Raw HTTP bodies, MCP arguments, and
auth tokens are not stored.

## Verification Targets

Useful focused tests for these controls include:

- `claim_enforcement_can_reject_or_warn_on_unclaimed_lane_paths`
- `sparse_lane_path_enforcement_blocks_patch_and_record_outside_selected_paths`
- `lane_patch_rejects_hardened_paths_and_quota_violations`
- `lane_event_and_trace_payload_limits_are_enforced`
- `lane_patch_can_replace_stable_line_with_expected_text`
- `lane_workdir_record_preview_reports_risks_and_oversized_files`
- `lane_workdir_sync_refuses_dirty_and_force_refreshes`
- `merge_lane_and_queue_enforce_readiness_blockers`
- `lane_merge_queue_explain_reports_dry_run_conflicts_without_recording_conflict_state`
- `conflict_explanations_classify_common_non_line_conflicts`
- `local_lane_http_api_can_require_bearer_token`
- `local_http_bodyless_mutations_reject_request_bodies`
- `external_http_and_mcp_mutations_emit_audit_events`
- `mcp_status_read_only_does_not_refresh_worktree_index`

## Related Reference

- [Configuration reference](../reference/config.md)
- [Lane work model](../lanes/work-model.md)
- [Structured patches](../lanes/structured-patches.md)
- [Spawn and materialize workdirs](../lanes/spawn-and-materialize-workdirs.md)
- [Readiness gates and merge safety](../concepts/readiness-gates-and-merge-safety.md)
- [HTTP API](../reference/http-api.md)
- [MCP tools](../reference/mcp-tools.md)
