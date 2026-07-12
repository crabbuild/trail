# Lane Merge HTTP Cutover Design

## Goal

Make the public HTTP API use the same lane-first hierarchy as the CLI:
`trail lane merge` maps to `POST /v1/lanes/{lane}/merge`.

## Scope

The canonical request is:

```http
POST /v1/lanes/{lane}/merge
Content-Type: application/json

{
  "into": "main",
  "strategy": "line-id-aware",
  "dry_run": true,
  "direct": false
}
```

`into` is required at the HTTP boundary. The CLI continues supplying its
existing default of `main` before issuing this request. The lane identifier is
always in the path; request-body aliases such as `lane` and `lane_id` are
removed.

The old `POST /v1/branches/{branch}/merge-lane` endpoint is removed with no
compatibility route. OpenAPI, audit target extraction, the daemon RPC client,
direct HTTP tests, reference documentation, and scale-benchmark HTTP calls
all use the replacement route.

## Deliberate Non-Changes

The persisted domain model remains unchanged. `OperationKind::LaneMerge`, the
`lane_merge` change-id namespace, and the `merge_queue` table already model
domain concepts rather than a deprecated command spelling. Renaming them would
needlessly invalidate historical data or require a storage migration without
changing the public contract.

Internal Rust methods such as `merge_lane_with_options` likewise remain
verb-object domain APIs. They are not CLI aliases and stay idiomatic Rust.

## Error Handling and Security

The replacement endpoint keeps the existing merge-strategy validation,
readiness checks, direct-merge guard, conflict behavior, audit lane/target
attribution, status codes, and JSON merge report. Requests to the removed
route follow the server's normal unknown-route behavior.

## Verification

Tests must prove the new path accepts a lane path segment and `into` body
field, the old path is not routable, the daemon-backed CLI calls the new route,
and OpenAPI documents only the new operation. Existing CLI parser and merge
flow coverage remains green.
