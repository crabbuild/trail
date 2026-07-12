# Lane Merge HTTP Cutover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the legacy branch-scoped lane-merge HTTP API with the lane-scoped hard-cutover endpoint used by `trail lane merge`.

**Architecture:** The new route identifies the lane in `/v1/lanes/{lane}/merge` and receives the target branch in a required `into` JSON field. The daemon client becomes a direct consumer of that contract. Storage and operation-kind names are intentionally unchanged.

**Tech Stack:** Rust, serde, existing Trail HTTP router/OpenAPI generator, Rust integration tests, shell benchmark script, Markdown docs.

## Global Constraints

- Remove `/v1/branches/{branch}/merge-lane`; do not retain a compatibility route.
- Canonical request fields are `into`, `strategy`, `dry_run`, and `direct`; the lane belongs only in the path.
- Retain `OperationKind::LaneMerge`, `lane_merge`, merge-queue storage, and domain-level `merge_lane*` Rust methods.
- Update client, server, audit, OpenAPI, tests, docs, and benchmark calls in the same change.

---

### Task 1: Define and route the lane-scoped HTTP contract

**Files:**
- Modify: `trail/src/server/request_types/collaboration.rs:7-19`
- Modify: `trail/src/server/route/lane/collaboration.rs:133-145`
- Modify: `trail/src/server/route/audit.rs:91-124`
- Test: `trail/tests/e2e.rs:10415-10431,19439-19450,19529-19548,19988-19992`

**Interfaces:**
- Consumes: `POST /v1/lanes/{lane}/merge` and `LaneMergeRequest { into, strategy, dry_run, direct }`.
- Produces: `MergeReport` with the existing readiness, direct-merge, and conflict behavior.

- [x] **Step 1: Write failing route tests**

```rust
let response = trail::server::handle_http_request(
    &mut db,
    &api_request(
        "POST",
        "/v1/lanes/doc-bot/merge",
        serde_json::json!({ "into": "main", "dry_run": true }),
    ),
);
assert_eq!(response.status, 200);

let legacy = trail::server::handle_http_request(
    &mut db,
    &api_request(
        "POST",
        "/v1/branches/main/merge-lane",
        serde_json::json!({ "lane": "doc-bot", "dry_run": true }),
    ),
);
assert_eq!(legacy.status, 400);
```

- [x] **Step 2: Verify the new route fails before implementation**

Run: `cargo test -p trail --test e2e merge_dry_run_reports_without_mutating_refs -- --exact --nocapture`

Expected: the new lane-scoped request is rejected because no route matches it.

- [x] **Step 3: Implement the replacement request and route**

```rust
pub(crate) struct LaneMergeRequest {
    pub(crate) into: String,
    #[serde(default)]
    pub(crate) strategy: Option<String>,
    #[serde(default, alias = "dry-run")]
    pub(crate) dry_run: bool,
    #[serde(default)]
    pub(crate) direct: bool,
}

if parts.len() == 4
    && parts[0] == "v1"
    && parts[1] == "lanes"
    && parts[3] == "merge"
    && request.method == "POST"
{
    let body: LaneMergeRequest = serde_json::from_slice(&request.body)?;
    validate_merge_strategy(body.strategy.as_deref())?;
    let lane = db.resolve_lane_handle(parts[2])?;
    let report = db.merge_lane_user_with_options(&lane, &body.into, body.dry_run, body.direct)?;
    return Ok(Some(json_response(200, "OK", &report)?));
}
```

Update audit extraction so this route's path supplies the lane and `body.into`
supplies the target branch ref. Delete the branch-scoped merge-lane cases.

- [x] **Step 4: Verify the server contract**

Run: `cargo test -p trail --test e2e merge_dry_run_reports_without_mutating_refs -- --exact --nocapture && cargo test -p trail --test e2e local_api_direct_merge_lane_conflict_records_conflict_set -- --exact --nocapture`

Expected: the new endpoint succeeds, the legacy route follows normal unknown-route handling (400), and conflict behavior is unchanged.

### Task 2: Cut over first-party consumers and generated API description

**Files:**
- Modify: `trail/src/cli/command/handler/daemon_rpc.rs:238-249`
- Modify: `trail/src/server/openapi/paths/collaboration.rs:118-122`
- Modify: `trail/src/server/openapi/schemas/collaboration.rs:5-18`
- Modify: `scripts/cli-scale-bench.sh:494-502`
- Test: `trail/tests/e2e.rs:12679-12689`

**Interfaces:**
- Consumes: `LaneMergeRequest` and `/v1/lanes/{lane}/merge` from Task 1.
- Produces: daemon CLI merge behavior and OpenAPI operation `laneMerge` with schema `LaneMergeRequest`.

- [x] **Step 1: Update the daemon route test to expect the new contract**

```rust
let merge = run_trail_json_daemon(
    temp.path(),
    &daemon_url,
    &["lane", "merge", "rpc-bot", "--into", "main", "--dry-run"],
);
assert_eq!(merge["dry_run"], true);
```

- [x] **Step 2: Verify the daemon test fails before the client change**

Run: `cargo test -p trail --test e2e cli_daemon_url_routes_hot_lane_commands -- --exact --nocapture`

Expected: the daemon client still posts to the removed route.

- [x] **Step 3: Cut over daemon, OpenAPI, and benchmark callers**

```rust
let body = serde_json::json!({
    "into": args.into,
    "strategy": args.strategy,
    "dry_run": args.dry_run,
    "direct": args.direct,
});
let report: MergeReport = client.post_json(
    &format!("/v1/lanes/{}/merge", args.name),
    &body,
)?;
```

Expose `/v1/lanes/{lane}/merge` as `laneMerge` with required path `lane` and
schema field `into`. Change the benchmark JSON from `lane_id` to `into` and
the URL to `/v1/lanes/daemonbot/merge`.

- [x] **Step 4: Verify daemon and OpenAPI consumers**

Run: `cargo test -p trail --test e2e cli_daemon_url_routes_hot_lane_commands -- --exact --nocapture`

Expected: PASS.

### Task 3: Update public references and run regression checks

**Files:**
- Modify: `docs/reference/http-api.md:170-186`
- Modify: `docs/integrations/openapi.md:28-31`
- Modify: `CHANGELOG.md`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Consumes: the endpoint and schema from Tasks 1 and 2.
- Produces: all public documentation referring only to the lane-scoped endpoint.

- [x] **Step 1: Replace API references and add the breaking-change note**

```markdown
| POST | `/v1/lanes/{lane_or_id}/merge` | Dry-run or explicitly direct-merge this lane into `into`. |
```

Document the request body field `into` and state that the previous
branch-scoped endpoint has been removed.

- [x] **Step 2: Check no legacy API contract remains**

Run: `! rg -n '/v1/branches/\{branch\}/merge-lane|/v1/branches/main/merge-lane|MergeLaneRequest|branchMergeLane' trail docs scripts README.md`

Expected: no matches.

- [x] **Step 3: Run formatting and focused regressions**

Run: `cargo fmt --all -- --check && cargo check -p trail && cargo test -p trail --bin trail && cargo test -p trail --test terminal_output_guard && cargo test -p trail --test e2e merge_dry_run_reports_without_mutating_refs -- --exact && cargo test -p trail --test e2e cli_daemon_url_routes_hot_lane_commands -- --exact`

Expected: every command passes.
