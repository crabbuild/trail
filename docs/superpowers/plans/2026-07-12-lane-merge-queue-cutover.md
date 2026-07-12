# Lane Merge Queue Hard Cutover Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace Trail's generic merge queue with a lane-only queue exposed as `trail lane merge-queue` and `/v1/lanes/merges/queue`, with a destructive v16 schema cutover.

**Architecture:** Queue state stores stable lane ids and resolves the current lane branch only when explaining or executing an entry. The CLI, HTTP, daemon, MCP, reports, renderers, storage methods, and documentation cut over together; generic branch/ref merging remains solely under `trail merge`.

**Tech Stack:** Rust, clap, serde, rusqlite/SQLite migrations, Trail's JSON HTTP server and OpenAPI generator, MCP JSON-RPC, Rust e2e tests, shell benchmarks, Markdown documentation.

## Global Constraints

- Remove `trail merge-queue`, `/v1/merge-queue`, `trail.merge_queue_*`, and `trail://workspace/merge-queue` without aliases.
- Accept only existing lanes in the queue; generic branches and refs continue through `trail merge`.
- Store `lane_id`, not a generic queue `source_ref`, and generate `lmq_` ids.
- Migrate schema v15 to v16 transactionally by discarding legacy queue rows and clearing legacy merge-result queue links.
- Preserve readiness, approval, gate, dirty-workdir, priority, serialization, cancellation, and conflict-pausing behavior.
- Preserve generic `source_ref` and `target_ref` fields in merge results and conflict sets.

---

### Task 1: Cut Storage and Domain Models Over to a Lane Queue

**Files:**
- Modify: `trail/src/db/mod.rs`
- Modify: `trail/src/db/storage/schema/ddl.rs`
- Modify: `trail/src/db/util/rows.rs`
- Modify: `trail/src/db/merge/queue.rs`
- Modify: `trail/src/db/merge/queue_store.rs`
- Modify: `trail/src/db/lane/identity.rs`
- Modify: `trail/src/db/core/doctor_activity.rs`
- Modify: `trail/src/model/reports/merge.rs`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Consumes: `Trail::lane_details(&str)`/existing lane selector resolution and `Trail::merge_lane_unlocked`.
- Produces: `enqueue_lane_merge`, `list_lane_merge_queue`, `explain_lane_merge_queue`, `run_lane_merge_queue`, `remove_lane_merge_queue`, and `LaneMergeQueue*` reports.

- [ ] **Step 1: Write failing v16 migration and lane-only queue tests**

Add focused e2e coverage equivalent to:

```rust
#[test]
fn schema_v16_discards_generic_merge_queue() {
    let (_temp, mut db) = initialized_db();
    db.enqueue_merge("doc-bot", "main", 0).unwrap();
    let conn = db.connection();
    conn.execute_batch("PRAGMA user_version = 15;").unwrap();
    drop(db);

    let reopened = Trail::open(_temp.path()).unwrap();
    assert_eq!(reopened.schema_user_version().unwrap(), 16);
    assert!(table_exists(reopened.connection(), "lane_merge_queue"));
    assert!(!table_exists(reopened.connection(), "merge_queue"));
    assert!(reopened.list_lane_merge_queue().unwrap().is_empty());
}

#[test]
fn lane_merge_queue_rejects_branch_sources() {
    let (_temp, mut db) = initialized_db();
    let error = db.enqueue_lane_merge("main", "main", 0).unwrap_err();
    assert!(error.to_string().contains("lane"));
}
```

Use the repository's actual test fixtures and direct SQLite helpers rather than introducing a second fixture style.

- [ ] **Step 2: Run the focused tests and confirm red state**

Run: `cargo test -p trail --test e2e schema_v16_discards_generic_merge_queue -- --exact --nocapture && cargo test -p trail --test e2e lane_merge_queue_rejects_branch_sources -- --exact --nocapture`

Expected: FAIL because schema version 16, `lane_merge_queue`, and lane-specific methods do not exist.

- [ ] **Step 3: Implement v16 schema and lane-specific report types**

Set:

```rust
const TRAIL_SCHEMA_VERSION: i64 = 16;
```

Replace the queue table with:

```sql
CREATE TABLE IF NOT EXISTS lane_merge_queue (
    queue_id TEXT PRIMARY KEY,
    lane_id TEXT NOT NULL,
    target_ref TEXT NOT NULL,
    status TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS lane_merge_queue_active_idx
    ON lane_merge_queue(lane_id, target_ref, status);
CREATE INDEX IF NOT EXISTS lane_merge_queue_run_idx
    ON lane_merge_queue(status, priority DESC, created_at ASC);
```

When upgrading from a version below 16, drop `merge_queue`, create the new table and indexes, rename `merge_results.queue_id` to `lane_queue_id`, and set existing non-null links to `NULL` inside the existing migration savepoint.

Define the entry shape:

```rust
pub struct LaneMergeQueueEntry {
    pub queue_id: String,
    pub lane_id: String,
    pub lane: String,
    pub target_ref: String,
    pub status: String,
    pub priority: i64,
    pub created_at: i64,
    pub updated_at: i64,
}
```

Rename all queue report types consistently and update SQLite row mapping to join `lane_merge_queue` with `lanes` for the human-facing lane name.

- [ ] **Step 4: Implement lane-only queue operations**

Use signatures:

```rust
pub fn enqueue_lane_merge(
    &mut self,
    lane: &str,
    target: &str,
    priority: i64,
) -> Result<LaneMergeQueueAddReport>;
pub fn list_lane_merge_queue(&self) -> Result<Vec<LaneMergeQueueEntry>>;
pub fn explain_lane_merge_queue(&mut self, selector: &str) -> Result<LaneMergeQueueExplainReport>;
pub fn run_lane_merge_queue(&mut self, limit: Option<usize>) -> Result<LaneMergeQueueRunReport>;
pub fn remove_lane_merge_queue(&mut self, selector: &str) -> Result<LaneMergeQueueRemoveReport>;
```

Resolve the lane before inserting, seed ids from `lane_id:target_ref:priority:now`, prefix ids with `lmq_`, and resolve `lane_id` again before explain/run. Remove branch fallbacks from queue normalization and execution.

- [ ] **Step 5: Run storage and safety regressions**

Run: `cargo test -p trail --test e2e schema_v16_discards_generic_merge_queue -- --exact && cargo test -p trail --test e2e lane_merge_queue_rejects_branch_sources -- --exact && cargo test -p trail --test e2e merge_lane_and_queue_enforce_readiness_blockers -- --exact && cargo test -p trail --test e2e merge_queue_pauses_on_conflict -- --exact`

Expected: PASS after renaming the existing queue tests to their lane-specific names and assertions.

- [ ] **Step 6: Commit the storage cutover**

```bash
git add trail/src/db trail/src/model/reports/merge.rs trail/tests/e2e.rs
git commit -m "refactor(trail)!: make merge queue lane-only"
```

### Task 2: Nest the CLI Queue Under `trail lane`

**Files:**
- Modify: `trail/src/cli/command.rs`
- Modify: `trail/src/cli/command/lane_args.rs`
- Modify: `trail/src/cli/command/collaboration_args.rs`
- Modify: `trail/src/cli/command/collaboration_args/merge.rs`
- Modify: `trail/src/cli/command/handler.rs`
- Modify: `trail/src/cli/command/handler/lane.rs`
- Modify: `trail/src/cli/command/handler/collaboration.rs`
- Modify: `trail/src/cli/command/handler/daemon_rpc.rs`
- Modify: `trail/src/cli/command/render/collaboration/merge.rs`
- Modify: `trail/src/db/merge/lane.rs`
- Test: `trail/src/cli/command.rs`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Consumes: Task 1's lane queue methods and reports.
- Produces: `trail lane merge-queue add|list|explain|run|remove`; the top-level spelling is unparseable.

- [ ] **Step 1: Write failing parser tests for the hard CLI cutoff**

```rust
#[test]
fn parses_lane_merge_queue_and_rejects_top_level_form() {
    let cli = Cli::try_parse_from([
        "trail", "lane", "merge-queue", "add", "doc-bot", "--into", "main",
    ]).expect("lane merge queue should parse");
    assert!(matches!(cli.command, Command::Lane(_)));
    assert!(Cli::try_parse_from([
        "trail", "merge-queue", "add", "doc-bot", "--into", "main",
    ]).is_err());
}
```

- [ ] **Step 2: Confirm the parser test fails**

Run: `cargo test -p trail --bin trail parses_lane_merge_queue_and_rejects_top_level_form -- --exact --nocapture`

Expected: FAIL because `merge-queue` is still top-level.

- [ ] **Step 3: Move queue args and dispatch into the lane command group**

Add:

```rust
pub(super) enum LaneSubcommand {
    // existing variants
    MergeQueue(LaneMergeQueueCommand),
}
```

Rename the argument structs to `LaneMergeQueueCommand`,
`LaneMergeQueueSubcommand`, and `LaneMergeQueue*Args`; change `source` to
`lane`. Delete `Command::MergeQueue` and its top-level dispatch arm. Route local
and daemon-backed lane dispatch through the Task 1 methods and Task 3 HTTP
paths.

- [ ] **Step 4: Update human output and direct-merge next steps**

Rename renderer functions to `render_lane_merge_queue_*`, display `Lane`
instead of `Source`, and emit only:

```text
trail lane merge-queue explain <queue-id>
trail lane merge-queue list
trail lane merge-queue add <lane> --into <target>
trail lane merge-queue run
```

- [ ] **Step 5: Verify local CLI behavior**

Run: `cargo test -p trail --bin trail parses_lane_merge_queue_and_rejects_top_level_form -- --exact && cargo test -p trail --test e2e lane_merge_queue_runs_lane_branch_into_main -- --exact && cargo test -p trail --test terminal_output_guard`

Expected: PASS with no callable top-level queue command.

- [ ] **Step 6: Commit the CLI cutover**

```bash
git add trail/src/cli trail/src/db/merge/lane.rs trail/tests
git commit -m "refactor(cli)!: nest merge queue under lanes"
```

### Task 3: Replace the HTTP and OpenAPI Contract

**Files:**
- Modify: `trail/src/server/request_types/collaboration.rs`
- Modify: `trail/src/server/route/lane/collaboration.rs`
- Modify: `trail/src/server/route/audit.rs`
- Modify: `trail/src/server/openapi/paths/collaboration.rs`
- Modify: `trail/src/server/openapi/schemas/collaboration.rs`
- Modify: `trail/src/cli/command/handler/daemon_rpc.rs`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Consumes: Task 1 report/method names and Task 2 daemon arguments.
- Produces: only `/v1/lanes/merges/queue` routes with strict lane-specific JSON.

- [ ] **Step 1: Write failing HTTP cutoff tests**

Exercise:

```rust
let add = api_request(
    "POST",
    "/v1/lanes/merges/queue",
    serde_json::json!({"lane": "doc-bot", "into": "main", "priority": 10}),
);
assert_eq!(handle_http_request(&mut db, &add).status, 200);

let legacy = api_request(
    "POST",
    "/v1/merge-queue",
    serde_json::json!({"source": "doc-bot", "target": "main"}),
);
assert_eq!(handle_http_request(&mut db, &legacy).status, 400);
```

Also assert strict rejection of `source`, `target`, and `target_branch` on the
new route.

- [ ] **Step 2: Confirm the route test fails**

Run: `cargo test -p trail --test e2e local_api_drives_lane_merge_queue -- --exact --nocapture`

Expected: FAIL because the new route is not registered.

- [ ] **Step 3: Implement strict requests and replacement routes**

```rust
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LaneMergeQueueAddRequest {
    pub(crate) lane: String,
    pub(crate) into: String,
    #[serde(default)]
    pub(crate) priority: i64,
}
```

Route add/list/run/explain/remove under `/v1/lanes/merges/queue`, delete every
`/v1/merge-queue` branch, update audit target extraction, and make the daemon
client call only the replacement routes.

- [ ] **Step 4: Replace OpenAPI paths and schemas**

Define `LaneMergeQueueAddRequest`, `LaneMergeQueueEntry`, and lane-specific
report schemas. Expose only:

```text
/v1/lanes/merges/queue
/v1/lanes/merges/queue/run
/v1/lanes/merges/queue/{selector}/explain
/v1/lanes/merges/queue/{selector}
```

- [ ] **Step 5: Verify HTTP, daemon, and OpenAPI behavior**

Run: `cargo test -p trail --test e2e local_api_drives_lane_merge_queue -- --exact && cargo test -p trail --test e2e cli_daemon_url_routes_hot_lane_commands -- --exact`

Expected: PASS and old routes return standard unknown-route responses.

- [ ] **Step 6: Commit the API cutover**

```bash
git add trail/src/server trail/src/cli/command/handler/daemon_rpc.rs trail/tests/e2e.rs
git commit -m "refactor(api)!: expose lane merge queue routes"
```

### Task 4: Cut MCP Over to Lane-Specific Tools and Resource

**Files:**
- Modify: `trail/src/mcp/audit.rs`
- Modify: `trail/src/mcp/capabilities/resources.rs`
- Modify: `trail/src/mcp/tool_call/merge.rs`
- Modify: `trail/src/mcp/tools/merge.rs`
- Modify: `trail/src/mcp/tools/annotations.rs`
- Modify: `trail/src/mcp/types/merge.rs`
- Modify: `trail/src/mcp/types/constants.rs`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Consumes: Task 1 methods and report types.
- Produces: `trail.lane_merge_queue_*` tools and `trail://workspace/lane-merge-queue`.

- [ ] **Step 1: Write failing MCP discovery and call assertions**

```rust
assert!(tools.iter().any(|tool| tool["name"] == "trail.lane_merge_queue_add"));
assert!(!tools.iter().any(|tool| tool["name"] == "trail.merge_queue_add"));
assert!(resources.iter().any(|resource| {
    resource["uri"] == "trail://workspace/lane-merge-queue"
}));
```

Call add with `{"lane":"doc-bot","target":"main","priority":0}` and assert
the structured content contains the stable `lane_id` and `lane` name.

- [ ] **Step 2: Confirm MCP tests fail**

Run: `cargo test -p trail --test e2e local_api_and_mcp_drive_lane_merge_queue_and_conflicts -- --exact --nocapture`

Expected: FAIL because discovery still exposes generic queue names.

- [ ] **Step 3: Rename tools, arguments, dispatch, annotations, audit, and resource**

Use exact names:

```text
trail.lane_merge_queue_add
trail.lane_merge_queue_list
trail.lane_merge_queue_explain
trail.lane_merge_queue_run
trail.lane_merge_queue_remove
trail://workspace/lane-merge-queue
```

The add arguments are `lane`, `target`, and optional `priority`; no `source`
alias remains. Preserve current read/write/destructive annotations.

- [ ] **Step 4: Verify MCP behavior**

Run: `cargo test -p trail --test e2e local_api_and_mcp_drive_lane_merge_queue_and_conflicts -- --exact`

Expected: PASS with old names absent.

- [ ] **Step 5: Commit the MCP cutover**

```bash
git add trail/src/mcp trail/tests/e2e.rs
git commit -m "refactor(mcp)!: expose lane merge queue tools"
```

### Task 5: Update First-Party References and Run the Full Verification Gate

**Files:**
- Modify: `README.md`
- Modify: `CHANGELOG.md`
- Modify: `docs/reference/http-api.md`
- Modify: `docs/reference/cli/lanes.md`
- Modify: `docs/guides/branch-checkout-and-merge.md`
- Modify: `docs/guides/hardening-agent-workflows.md`
- Modify: `docs/getting-started/first-lane-workflow.md`
- Modify: `docs/getting-started/install-and-build.md`
- Modify: `docs/concepts/readiness-gates-and-merge-safety.md`
- Modify: `docs/lanes/handoff-review-and-merge.md`
- Modify: `skills/use-trail/SKILL.md`
- Modify: `skills/use-trail/references/lanes.md`
- Modify: `skills/use-trail/references/safety-and-recovery.md`
- Modify: `skills/use-trail/references/integrations.md`
- Modify: `scripts/cli-scale-bench.sh`
- Modify: `scripts/check-cli-scale-thresholds.py`
- Test: repository-wide search and Trail test suites

**Interfaces:**
- Consumes: all replacement public names from Tasks 1-4.
- Produces: repository guidance, examples, and benchmarks containing no callable legacy queue surface.

- [ ] **Step 1: Replace public examples and benchmark calls**

Use:

```sh
trail lane merge-queue add doc-bot --into main
trail lane merge-queue explain doc-bot
trail lane merge-queue run
```

Use `/v1/lanes/merges/queue` for HTTP examples and daemon scale calls. Rename
benchmark metric labels to `lane_merge_queue_*` so reports match the public
domain.

- [ ] **Step 2: Assert no callable legacy surface remains**

Run:

```bash
rg -n 'trail merge-queue|/v1/merge-queue|trail\.merge_queue_|trail://workspace/merge-queue' \
  README.md CHANGELOG.md docs skills scripts trail/src \
  --glob '!docs/superpowers/specs/2026-07-12-lane-merge-queue-cutover-design.md' \
  --glob '!docs/superpowers/plans/2026-07-12-lane-merge-queue-cutover.md'
```

Expected: only explicit negative parser, route, and tool assertions that prove
removal. Inspect every match; no help text, production route, callable tool,
resource, documentation example, or benchmark invocation may use a legacy
name.

- [ ] **Step 3: Format and run focused tests**

Run:

```bash
cargo fmt --all -- --check
cargo check -p trail
cargo test -p trail --bin trail
cargo test -p trail --test terminal_output_guard
cargo test -p trail --test e2e schema_v16_discards_generic_merge_queue -- --exact
cargo test -p trail --test e2e lane_merge_queue_rejects_branch_sources -- --exact
cargo test -p trail --test e2e lane_merge_queue_runs_lane_branch_into_main -- --exact
cargo test -p trail --test e2e local_api_drives_lane_merge_queue -- --exact
cargo test -p trail --test e2e local_api_and_mcp_drive_lane_merge_queue_and_conflicts -- --exact
```

Expected: every command passes.

- [ ] **Step 4: Run the complete Trail suite**

Run: `cargo test -p trail`

Expected: all unit, integration, e2e, and documentation tests pass.

- [ ] **Step 5: Commit documentation and benchmark cutover**

```bash
git add README.md CHANGELOG.md docs skills scripts
git commit -m "docs: document lane merge queue cutover"
```

- [ ] **Step 6: Record final evidence**

Run: `git status --short && git log -6 --oneline`

Expected: no tracked implementation changes remain uncommitted; unrelated
pre-existing untracked files may still be present. The recent commit list shows
the storage, CLI, API, MCP, documentation, design, and plan commits.
