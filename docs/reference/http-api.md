# HTTP API Reference

The daemon serves JSON HTTP routes under `/v1`.

## Auth

`GET /v1/health` is unauthenticated. Other routes require a token unless the daemon starts with `--no-auth`.

Headers:

```text
Authorization: Bearer <token>
x-crabdb-token: <token>
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
| GET | `/v1/lanes/{lane_or_id}/handoff` | Handoff packet. |
| GET | `/v1/lanes/{lane_or_id}/workdir` | Workdir path. |
| GET | `/v1/lanes/{lane_or_id}/diff` | Lane branch diff. |
| POST | `/v1/lanes/{lane_or_id}/read-file` | Read lane file. |
| POST | `/v1/lanes/{lane_or_id}/sync-workdir` | Sync workdir. |
| POST | `/v1/lanes/{lane_or_id}/record` | Record lane workdir. |
| POST | `/v1/lanes/{lane_or_id}/rewind` | Rewind lane branch. |
| POST | `/v1/lanes/{lane_or_id}/tests` | Run test gate. |
| POST | `/v1/lanes/{lane_or_id}/evals` | Run eval gate. |
| POST | `/v1/lanes/{lane_or_id}/patches` | Apply lane patch. |

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
| DELETE | `/v1/merge-queue/{selector}` | Remove queue item. |
| GET | `/v1/conflicts` | List conflicts. |
| GET | `/v1/conflicts/{conflict_set_id}?limit=50` | Show conflict with explanation evidence. |
| POST | `/v1/conflicts/{conflict_set_id}/resolve` | Resolve conflict. |
| POST | `/v1/branches/{branch}/merge-lane` | Merge lane branch. |

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

- OpenAPI paths: `crates/crabdb/src/server/openapi/paths`
- Route handlers: `crates/crabdb/src/server/route`
- Request types: `crates/crabdb/src/server/request_types`
