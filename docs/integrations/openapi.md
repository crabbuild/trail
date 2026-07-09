# OpenAPI

Trail exposes an OpenAPI 3.1 document for the local HTTP API.

## Print the Contract

```sh
trail api openapi
```

## Write the Contract

```sh
trail api openapi --output openapi.json
```

## Fetch from the Daemon

```sh
curl -H "Authorization: Bearer $TOKEN" \
  http://127.0.0.1:8765/v1/openapi.json
```

## Path Groups

The OpenAPI path builder groups routes as:

- Core: health, OpenAPI, doctor, status, record, diff, timeline, why, history, code-from, config, ignore, guardrails.
- Lanes: list/spawn/show/remove, status, review, contribution, gates, readiness, handoff, workdir, diff, read-file, sync-workdir, record, rewind, tests, evals, patches.
- Collaboration: sessions, approvals, leases, claims, anchors, merge queue, conflicts, merge-lane.
- Turns and traces: turns, messages, events, spans, runs.

## Code Facts Used

- OpenAPI builder: `crates/trail/src/server/openapi.rs`
- Paths: `crates/trail/src/server/openapi/paths`
- Schemas: `crates/trail/src/server/openapi/schemas`
- Tests: `local_api_and_cli_export_openapi_contract`
