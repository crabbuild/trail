# HTTP API Reference

The daemon serves JSON HTTP routes under `/v1`.
Accepted HTTP connections use 30-second read/write timeouts; slow requests
receive `408 Request Timeout` without stopping the listener. Total
requests larger than 16 MiB, including headers and body, are rejected before
routing.
Trail accepts origin-form `HTTP/1.0` or `HTTP/1.1` request lines and
fixed-length requests only: malformed request lines, malformed or unterminated
header lines, bare CR/LF request-head line endings, header names with
surrounding whitespace, missing header terminators, duplicate or non-decimal
`Content-Length`, bodies without `Content-Length`, body length mismatches, and
`Transfer-Encoding` requests are rejected before routing.
Origin-form request targets must start with `/` and cannot contain fragments,
control characters, or backslash separators.
Requests with an `Origin` header are accepted only for well-formed loopback
origins such as `localhost`, `127.0.0.1`, and `::1`; invalid ports and opaque
origins such as `null` are rejected.
Routed requests must also include a `Host` header with a loopback host such as
`localhost`, `127.0.0.1`, or `[::1]`; missing, malformed, non-loopback, and
invalid-port hosts are rejected before route handling.
Daemon listener connections are rate-limited per peer. The default limit is
600 requests per 60 seconds; excess requests receive `429 Too Many Requests`.

Mutation request bodies are strict JSON objects. Unknown fields are rejected
with `400 Bad Request`; they are not silently ignored. Top-level request
schemas in `/v1/openapi.json` set `additionalProperties: false` to match that
runtime behavior; patch edit/file variants are also emitted as strict nested
schemas.

For retry-safe mutation requests, send:

```text
Idempotency-Key: <stable unique key>
```

Trail replays the first stored response when the same key is reused with the
same method, path, and body. Reusing the key for different request content
returns `400 Bad Request`. Keys must be 1-200 non-control characters.
Unauthorized and forbidden responses are not cached.
Replayed mutation responses still produce audit rows, marked with
`idempotency_replay: true` in the audit summary.

Non-GET mutation attempts, including unauthorized and forbidden attempts, are
recorded in the local `external_mutation_audit` table with actor label,
method/path, status, inferred lane/ref when available, and a small redacted
result summary. Turn-scoped mutation failures can still be attributed through
the turn id in the request path. Actor labels identify the HTTP auth path, such
as `http:bearer`, `http:x-trail-token`, or `http:no-auth`; raw request bodies
and tokens are not stored.

## Auth

`GET /v1/health` is unauthenticated. Other routes require a token unless the daemon starts with `--no-auth`.

Headers:

```text
Authorization: Bearer <token>
x-trail-token: <token>
```

## Core Routes

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/v1/health` | Service liveness. |
| GET | `/v1/openapi.json` | OpenAPI document. |
| GET | `/v1/doctor` | Workspace diagnostics. |
| GET | `/v1/status` | Current status. |
| POST | `/v1/record` | Record workspace changes. |
| GET | `/v1/diff` | Diff range, roots, or dirty worktree. |
| GET | `/v1/timeline` | Recent operations. |
| GET | `/v1/why` | Line provenance. |
| GET | `/v1/history` | File or line history. |
| GET | `/v1/code-from` | Trace code from selector. |
| GET | `/v1/config` | List config. |
| POST | `/v1/config` | Set config. |
| GET | `/v1/config/{key}` | Get config. |
| GET | `/v1/ignore` | List ignore rules. |
| POST | `/v1/ignore/patterns` | Add ignore rule. |
| DELETE | `/v1/ignore/patterns` | Remove ignore rule. |
| POST | `/v1/ignore/check` | Check ignored path. |
| POST | `/v1/guardrails/check` | Preflight guardrails. |

## Lane Routes

| Method | Path | Purpose |
| --- | --- | --- |
| GET | `/v1/lanes` | List lanes. |
| POST | `/v1/lanes` | Spawn lane. |
| GET | `/v1/lanes/{lane_or_id}` | Show lane. |
| DELETE | `/v1/lanes/{lane_or_id}` | Remove lane. |
| GET | `/v1/lanes/{lane_or_id}/status` | Lane status. |
| GET | `/v1/lanes/{lane_or_id}/review` | Review packet. |
| GET | `/v1/lanes/{lane_or_id}/contribution` | Review bundle. |
| GET | `/v1/lanes/{lane_or_id}/gates` | Gate history. |
| GET | `/v1/lanes/{lane_or_id}/readiness` | Merge readiness. |
| GET | `/v1/lanes/{lane_or_id}/refresh-preview?target=main` | Preview refreshing lane onto target branch. |
| GET | `/v1/lanes/{lane_or_id}/handoff` | Handoff packet. |
| GET | `/v1/lanes/{lane_or_id}/workdir` | Workdir path. |
| GET | `/v1/lanes/{lane_or_id}/diff` | Lane branch diff. |
| POST | `/v1/lanes/{lane_or_id}/read-file` | Read lane file. |
| POST | `/v1/lanes/{lane_or_id}/hydrate` | Hydrate sparse workdir paths. |
| POST | `/v1/lanes/{lane_or_id}/sync-workdir` | Sync workdir. |
| POST | `/v1/lanes/{lane_or_id}/record` | Record lane workdir. |
| POST | `/v1/lanes/{lane_or_id}/rewind` | Rewind lane branch. |
| POST | `/v1/lanes/{lane}/merge` | Dry-run or explicitly direct-merge this lane into body field `into`. |
| POST | `/v1/lanes/{lane_or_id}/tests` | Run test gate. |
| POST | `/v1/lanes/{lane_or_id}/evals` | Run eval gate. |
| POST | `/v1/lanes/{lane_or_id}/patches` | Apply lane patch. |

Patch requests accept either native `edits` or compatibility `files`; provide
one non-empty array, not both.

`GET /v1/lanes/{lane_or_id}/status` includes `base_status` when Trail can
resolve the workspace default branch. It reports the target branch/ref, target
change, lane base change, `operations_behind`, and whether the lane base is
stale.

`POST /v1/lanes` accepts `workdir_mode` values `virtual`, `sparse`,
`full-cow`, `overlay-cow`, and `nfs-cow`. `virtual` creates no workdir, `sparse` requires
`paths`, and `full-cow` creates a full materialized workdir using filesystem
clone COW when available. `overlay-cow` creates an empty workdir mountpoint and
records an overlay backend; a runtime such as `trail agent start
--workdir-mode overlay-cow` mounts the FUSE view and keeps it alive while the
agent runs. The response includes `workdir_mode`, `cow_backend`, `sparse_paths`,
and `overlay_available`.
On macOS, `nfs-cow` reports `cow_backend: "nfs-overlay"` and requires no
macFUSE installation.

`POST /v1/lanes/{lane_or_id}/hydrate` accepts the same body as path-scoped
`sync-workdir`, but requires at least one `paths` entry.

`POST /v1/lanes/{lane_or_id}/sync-workdir` returns `rescue_workdir` when
`force=true` overwrites dirty materialized workdir files or replaces a
non-directory file at the lane workdir path. The rescue directory contains
copied recoverable regular files plus `manifest.json`.

`POST /v1/lanes/{lane_or_id}/record` accepts `{"preview": true}` to return a
non-committing record preview with changed paths, ignored paths, risky workdir
entries, oversized changed files, and policy allow/block details.

`GET /v1/lanes/{lane_or_id}/refresh-preview?target=<branch>` returns a
non-committing refresh/rebase preview with operations-behind, conflicts, changed
paths, and next steps.

`POST /v1/lanes/{lane_or_id}/merge` requires a JSON `into` target branch and
allows `dry_run=true` preflight. A non-dry-run merge into the workspace default
branch requires `direct=true`; otherwise enqueue with `POST /v1/merge-queue`
and run the queue. The former branch-scoped `merge-lane` endpoint was removed.

## Collaboration Routes

| Method | Path | Purpose |
| --- | --- | --- |
| GET/POST | `/v1/sessions` | List or start sessions. |
| GET | `/v1/sessions/current` | Current sessions. |
| GET | `/v1/sessions/{session_id}` | Show session. |
| GET | `/v1/sessions/{session_id}/context` | Session context. |
| POST | `/v1/sessions/{session_id}/end` | End session. |
| GET/POST | `/v1/approvals` | List or request approvals. |
| GET | `/v1/approvals/{approval_id}` | Show approval. |
| POST | `/v1/approvals/{approval_id}/decision` | Decide approval. |
| GET/POST | `/v1/leases` | List or acquire leases. |
| DELETE | `/v1/leases/{lease_id}` | Release lease. |
| POST | `/v1/lanes/{lane_or_id}/claims` | Claim path. |
| GET/POST | `/v1/anchors` | List or create anchors. |
| GET/DELETE | `/v1/anchors/{anchor_id}` | Resolve or delete anchor. |
| GET/POST | `/v1/merge-queue` | List or queue merge. |
| POST | `/v1/merge-queue/run` | Run queue. |
| GET | `/v1/merge-queue/explain?selector=<selector>` | Explain why a queue item is ready or blocked. |
| GET | `/v1/merge-queue/{selector}/explain` | Explain a queue item by path-safe selector. |
| DELETE | `/v1/merge-queue/{selector}` | Remove queue item. |
| GET | `/v1/conflicts` | List conflicts. |
| GET | `/v1/conflicts/{conflict_set_id}?limit=50` | Show conflict with explanation evidence. |
| POST | `/v1/conflicts/{conflict_set_id}/resolve` | Resolve conflict. |

Conflict explanations include the stored `base_root`, `target_root`, and
`source_root` snapshots used to reproduce the conflict, plus per-path
`conflict_class` values such as `modify/modify`, `delete/modify`,
`rename/modify`, `binary`, `mode`, and `same_insertion_gap`.
They also include `known_resolutions` for paths whose path/content conflict
signature matches a previously resolved conflict.
`POST /v1/conflicts/{conflict_set_id}/resolve` requires exactly one of `take`
or `manual`. Manual file values can be plain strings or objects with only
`content`, `delete`, and `executable`; unknown keys are rejected.

Bodyless mutation routes reject non-empty request bodies. This applies to
path-only deletes such as lane removal, lease release, anchor deletion, and
merge-queue removal.

## Turn and Trace Routes

| Method | Path | Purpose |
| --- | --- | --- |
| POST | `/v1/lane/turns` | Begin turn. |
| GET | `/v1/lane/events` | List events. |
| GET | `/v1/lane/spans` | List spans. |
| GET | `/v1/lane/spans/summary` | Span summary. |
| GET/POST | `/v1/lane/runs` | List or pause runs. |
| GET | `/v1/lane/runs/{run_id}` | Show run. |
| POST | `/v1/lane/runs/{run_id}/resume` | Resume run. |
| GET | `/v1/lane/spans/{span_id}` | Show span. |
| POST | `/v1/lane/spans/{span_id}/end` | End span. |
| GET | `/v1/lane/turns/{turn_id}` | Show turn. |
| POST | `/v1/lane/turns/{turn_id}/messages` | Add message. |
| POST | `/v1/lane/turns/{turn_id}/events` | Add event. |
| POST | `/v1/lane/turns/{turn_id}/spans` | Start span. |
| POST | `/v1/lane/turns/{turn_id}/patches` | Apply turn patch. |
| POST | `/v1/lane/turns/{turn_id}/end` | End turn. |

## Code Facts Used

- OpenAPI paths: `trail/src/server/openapi/paths`
- Route handlers: `trail/src/server/route`
- Request types: `trail/src/server/request_types`
