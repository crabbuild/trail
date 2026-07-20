# SQLite Lane Initialization Coordination Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the unmerged filesystem lane-initialization singleflight with SQLite-owned, process-liveness-fenced coordination that safely replays identical concurrent lane spawns at large-repository scale.

**Architecture:** Schema v20 adds one owner row per active lane initialization. Short `BEGIN IMMEDIATE` transactions claim, fence, heartbeat, transition, release, or take over an initialization; slow materialization remains outside SQLite transactions. Contenders poll durable state and may take over only after proving the recorded PID/start identity dead—never because time elapsed.

**Tech Stack:** Rust, rusqlite/SQLite WAL, Trail process-liveness helpers, existing lane materialization and observer lifecycle, Rust integration tests, Python/shell scale qualification.

## Global Constraints

- `lane_initializations` remains the durable authority for request identity and phase.
- The owner fence is the exact pair `(owner_token, owner_generation)`; the token is 64 lowercase hexadecimal characters and the generation is positive.
- Time or heartbeat age alone never permits takeover from a live or indeterminate owner.
- No SQLite write transaction may span materialization, environment setup, or observer startup.
- Terminal transitions leave zero owner rows.
- Coordination creates no `.lock`, `.anchor`, `.identity`, or candidate files.
- A quiescent workspace copy must remain usable without filesystem repair.
- Schema v19-to-v20 migration is transactional; v18 opens must migrate through v19 and then v20.
- `COMMITTED_REPAIR_REQUIRED` must report the actual durable phase even when repair-state persistence fails.
- A contender may wait from 10 ms up to 250 ms with bounded jitter; after 30 minutes it returns `LANE_INITIALIZATION_IN_PROGRESS` without revoking the owner.
- Execution override (2026-07-20): for Tasks 3-6, implement the scoped production change first, then add or update focused regression coverage and run verification. Do not use a red/green TDD cycle.
- Preserve the dirty `prolly` submodule and all unrelated untracked documentation.
- Do not merge, rebase, push, reset, clean, switch branches, or modify the primary worktree during implementation.

---

## File map

- `trail/src/db/storage/schema/ddl.rs`: exact schema-v20 owner table and schema-shape validation.
- `trail/src/db/storage/schema.rs`: v19-to-v20 migration and version validation.
- `trail/src/db/storage/mod.rs`: schema-v20 exports and test hooks.
- `trail/src/db/core/init.rs`: sequential v18→v19→v20 opening under schema-transition exclusion.
- `trail/src/db/mod.rs`: schema constants and current-version wiring.
- `trail/src/db/change_ledger/activation.rs`: checked current-schema activation digest/version.
- `trail/src/db/lane/initialization_owner.rs`: owner record, fence, claim/takeover, heartbeat, release, wait policy.
- `trail/src/db/lane/initialization.rs`: reservation and phase-transition integration with owner fences.
- `trail/src/db/lane/lifecycle.rs`: contender loop, slow-work boundaries, association fencing, and removal of filesystem singleflight.
- `trail/src/error.rs`: stable in-progress and lost-fence errors/codes.
- `trail/src/model/reports/maintenance.rs`, `trail/src/mcp/response.rs`: HTTP/MCP status and details.
- `trail/tests/schema_v20_lane_initialization_owners.rs`: migration, rollback, and exact shape.
- `trail/tests/lane_initialization_faults.rs`: concurrency, crash, takeover, fencing, release, and resource tests.
- `trail/tests/lane_initialization.rs`: public replay/conflict contracts.
- `trail/tests/fixtures/changed_path_raw_mutations.v1`: reviewed mutation inventory after filesystem code removal.
- `docs/reference/cli/lanes.md`: SQLite owner/wait/recovery contract.

---

### Task 1: Schema v20 owner authority and migration

**Files:**
- Modify: `trail/src/db/mod.rs`
- Modify: `trail/src/db/storage/schema/ddl.rs`
- Modify: `trail/src/db/storage/schema.rs`
- Modify: `trail/src/db/storage/mod.rs`
- Modify: `trail/src/db/core/init.rs`
- Modify: `trail/src/db/change_ledger/activation.rs`
- Create: `trail/tests/schema_v20_lane_initialization_owners.rs`

**Interfaces:**
- Produces: `LANE_INITIALIZATION_OWNERS_V20`, `validate_schema_v20`, `migrate_schema_v19_to_v20`, and test-only `SchemaV20MigrationBoundary` hooks.
- Consumes: existing v18→v19 migration, exact `sqlite_master` normalization, workspace schema-transition lock.

- [ ] **Step 1: Write failing exact-shape and migration tests**

Create tests that assert this exact table and migration behavior:

```rust
const EXPECTED_OWNER_COLUMNS: &[(&str, &str, bool, bool)] = &[
    ("initialization_id", "TEXT", false, true),
    ("owner_token", "TEXT", true, false),
    ("owner_generation", "INTEGER", true, false),
    ("owner_pid", "INTEGER", true, false),
    ("owner_process_start_identity", "TEXT", true, false),
    ("acquired_at", "INTEGER", true, false),
    ("heartbeat_at", "INTEGER", true, false),
];

#[test]
fn v19_open_migrates_owner_authority_atomically_to_v20() {
    let fixture = SchemaV20Fixture::from_v19();
    let db = fixture.open();
    assert_eq!(db.schema_user_version_for_test(), 20);
    assert_eq!(fixture.owner_columns(), EXPECTED_OWNER_COLUMNS);
    assert_eq!(fixture.owner_count(), 0);
}

#[test]
fn v18_open_runs_v19_backfill_then_v20_owner_migration() {
    let fixture = SchemaV20Fixture::from_v18_with_lane();
    let db = fixture.open();
    assert_eq!(db.schema_user_version_for_test(), 20);
    assert_eq!(fixture.initialization_count(), 1);
    assert_eq!(fixture.owner_count(), 0);
}

#[test]
fn v20_migration_fault_rolls_back_table_metadata_and_user_version() {
    let fixture = SchemaV20Fixture::from_v19();
    fixture.install_failure(SchemaV20MigrationBoundary::AfterDdlBeforeUserVersion);
    assert!(fixture.open_result().is_err());
    assert_eq!(fixture.raw_user_version(), 19);
    assert!(!fixture.table_exists("lane_initialization_owners"));
}
```

The test target defines this exact helper surface around a `tempfile::TempDir`,
the existing v18 fixture creator, and a raw rusqlite v19 connection:

```rust
struct SchemaV20Fixture { root: tempfile::TempDir, db_path: PathBuf }
impl SchemaV20Fixture {
    fn from_v19() -> Self;
    fn from_v18_with_lane() -> Self;
    fn open(&self) -> Trail;
    fn open_result(&self) -> trail::Result<Trail>;
    fn install_failure(&self, boundary: SchemaV20MigrationBoundary);
    fn raw_user_version(&self) -> i64;
    fn table_exists(&self, name: &str) -> bool;
    fn owner_columns(&self) -> Vec<(String, String, bool, bool)>;
    fn owner_count(&self) -> i64;
    fn initialization_count(&self) -> i64;
}
```

- [ ] **Step 2: Run the new target and verify RED**

```bash
cargo test -p trail --test schema_v20_lane_initialization_owners -- --nocapture
```

Expected: compilation fails because schema-v20 helpers and hooks do not exist.

- [ ] **Step 3: Add the exact v20 DDL and validator**

```rust
pub(super) const LANE_INITIALIZATION_OWNERS_V20: &str = r#"
CREATE TABLE lane_initialization_owners (
    initialization_id TEXT PRIMARY KEY
        REFERENCES lane_initializations(initialization_id) ON DELETE CASCADE,
    owner_token TEXT NOT NULL CHECK (
        length(owner_token)=64 AND owner_token NOT GLOB '*[^0-9a-f]*'),
    owner_generation INTEGER NOT NULL CHECK (owner_generation > 0),
    owner_pid INTEGER NOT NULL CHECK (owner_pid > 0),
    owner_process_start_identity TEXT NOT NULL
        CHECK (length(owner_process_start_identity) > 0),
    acquired_at INTEGER NOT NULL,
    heartbeat_at INTEGER NOT NULL
);
"#;
```

Set `TRAIL_SCHEMA_VERSION` to `20`, retain explicit constants for versions 18
and 19, and validate the owner table by comparing normalized `sqlite_master`
objects to an in-memory database created from the same DDL.

- [ ] **Step 4: Implement sequential transactional migration**

```rust
pub(crate) fn migrate_schema_v19_to_v20(conn: &mut Connection) -> Result<()> {
    validate_schema_v19_for_migration(conn)?;
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(ddl::LANE_INITIALIZATION_OWNERS_V20)?;
    fail_schema_v20_migration_if_installed(&tx)?;
    update_schema_version_metadata(&tx, 20)?;
    tx.pragma_update(None, "user_version", 20)?;
    validate_schema_v20(&tx)?;
    tx.commit()?;
    Ok(())
}
```

Add `update_schema_version_metadata(tx: &Transaction<'_>, version: i64) ->
Result<()>` in `schema.rs`. It updates both `schema.version` and `app.version`
with one captured `now_ts()` and is reused by both migration functions.

Make existing opening perform v18→v19 and then v19→v20 while holding the one
schema-transition exclusion. A v19 database runs only the second migration; a
v20 database only validates.

- [ ] **Step 5: Update activation/version evidence and run GREEN**

```bash
cargo test -p trail --test schema_v20_lane_initialization_owners -- --nocapture
cargo test -p trail --test schema_v19_lane_initializations -- --nocapture
cargo test -p trail --test schema_v18_hard_cutover -- --nocapture
cargo test -p trail db::change_ledger::activation::tests::unsupported_platform_and_incomplete_evidence_are_hard_off -- --exact --nocapture
```

Expected: all tests pass; migration failure leaves v19 valid.

- [ ] **Step 6: Commit Task 1**

```bash
git add trail/src/db/mod.rs trail/src/db/storage/schema/ddl.rs trail/src/db/storage/schema.rs trail/src/db/storage/mod.rs trail/src/db/core/init.rs trail/src/db/change_ledger/activation.rs trail/tests/schema_v20_lane_initialization_owners.rs
git commit -m "feat: add lane initialization owner schema"
```

---

### Task 2: SQLite owner claim, liveness, and generation fencing

**Files:**
- Create: `trail/src/db/lane/initialization_owner.rs`
- Modify: `trail/src/db/lane/mod.rs`
- Modify: `trail/src/db/lane/initialization.rs`
- Test: `trail/src/db/lane/initialization_owner.rs`

**Interfaces:**
- Produces `LaneInitializationFence`, `LaneInitializationClaim`, `claim_lane_initialization_owner`, `heartbeat_lane_initialization_owner`, `release_lane_initialization_owner`, and `owner_fence_matches`.
- Consumes `process_matches_start_token`, `current_process_start_token`, `LaneInitializationRecord`, and `ResolvedLaneSpawnRequest`.

- [ ] **Step 1: Write RED owner state-machine tests**

```rust
#[test]
fn first_claim_inserts_reservation_and_generation_one_owner_atomically() {
    let mut fixture = OwnerFixture::new();
    let claim = fixture.claim("lane-a");
    let LaneInitializationClaim::Owned { fence, resumed, .. } = claim else { panic!() };
    assert!(!resumed);
    assert_eq!(fence.owner_generation, 1);
    assert_eq!(fence.owner_token.len(), 64);
    assert_eq!(fixture.owner_count("lane-a"), 1);
}

#[test]
fn live_owner_is_contended_even_when_heartbeat_is_expired() {
    let mut fixture = OwnerFixture::new();
    let first = fixture.claim("lane-a").owned_fence();
    fixture.age_heartbeat("lane-a", i64::MIN / 2);
    assert!(matches!(fixture.claim("lane-a"), LaneInitializationClaim::Contended { .. }));
    assert_eq!(fixture.stored_fence("lane-a"), first);
}

#[test]
fn dead_owner_takeover_is_cas_fenced_and_increments_generation() {
    let mut fixture = OwnerFixture::new();
    let first = fixture.claim("lane-a").owned_fence();
    fixture.mark_owner_dead("lane-a");
    let second = fixture.claim("lane-a").owned_fence();
    assert_ne!(second.owner_token, first.owner_token);
    assert_eq!(second.owner_generation, first.owner_generation + 1);
    assert!(!fixture.owner_fence_matches("lane-a", &first));
}

#[test]
fn pid_reuse_with_a_different_start_identity_is_takeover_not_contention() {
    let mut fixture = OwnerFixture::new();
    fixture.install_owner(std::process::id(), "different-start-token");
    assert!(matches!(
        fixture.claim("lane-a"),
        LaneInitializationClaim::Owned { resumed: true, .. }
    ));
}

#[test]
fn terminal_initialization_replays_without_creating_an_owner() {
    let mut fixture = OwnerFixture::new();
    fixture.install_terminal("lane-a", LaneInitializationPhase::ObserverReady);
    assert!(matches!(fixture.claim("lane-a"), LaneInitializationClaim::Terminal(_)));
    assert_eq!(fixture.owner_count("lane-a"), 0);
}
```

The unit-test module defines this exact helper surface around a fresh v20
in-memory `Trail` and deterministic process-liveness overrides:

```rust
struct OwnerFixture { db: Trail, request: ResolvedLaneSpawnRequest }
impl OwnerFixture {
    fn new() -> Self;
    fn claim(&mut self, lane: &str) -> LaneInitializationClaim;
    fn age_heartbeat(&self, lane: &str, heartbeat: i64);
    fn mark_owner_dead(&self, lane: &str);
    fn install_owner(&self, pid: u32, start_identity: &str);
    fn install_terminal(&self, lane: &str, phase: LaneInitializationPhase);
    fn owner_count(&self, lane: &str) -> i64;
    fn stored_fence(&self, lane: &str) -> LaneInitializationFence;
    fn owner_fence_matches(&self, lane: &str, fence: &LaneInitializationFence) -> bool;
}
```

- [ ] **Step 2: Run RED**

```bash
cargo test -p trail --lib db::lane::initialization_owner::tests -- --nocapture
```

Expected: compilation fails because the module and claim API do not exist.

- [ ] **Step 3: Implement exact owner interfaces**

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LaneInitializationFence {
    pub(crate) owner_token: String,
    pub(crate) owner_generation: i64,
}

#[derive(Clone, Debug)]
pub(crate) enum LaneInitializationClaim {
    Owned {
        record: LaneInitializationRecord,
        fence: LaneInitializationFence,
        resumed: bool,
    },
    Contended {
        record: LaneInitializationRecord,
        owner_pid: u32,
    },
    Terminal(LaneInitializationRecord),
}
```

Generate tokens from 32 OS-random bytes and lowercase hex. Treat a live PID with
an unavailable start-token lookup as live. Probe liveness outside the write
transaction, then compare the complete old owner tuple during CAS takeover.

- [ ] **Step 4: Implement atomic claim and takeover**

```rust
pub(crate) fn claim_lane_initialization_owner(
    db: &mut Trail,
    request: &ResolvedLaneSpawnRequest,
) -> Result<LaneInitializationClaim>;
```

Insert reservation and owner atomically. Replace a proven-dead owner only with:

```sql
UPDATE lane_initialization_owners
SET owner_token=?1, owner_generation=owner_generation+1,
    owner_pid=?2, owner_process_start_identity=?3,
    acquired_at=?4, heartbeat_at=?4
WHERE initialization_id=?5
  AND owner_token=?6 AND owner_generation=?7
  AND owner_pid=?8 AND owner_process_start_identity=?9
```

Zero changed rows restart claim; they never surface as uniqueness/SQLite errors.

- [ ] **Step 5: Implement heartbeat, release, and fence checks**

```rust
pub(crate) fn heartbeat_lane_initialization_owner(
    conn: &Connection,
    initialization_id: &str,
    fence: &LaneInitializationFence,
) -> Result<()>;

pub(crate) fn release_lane_initialization_owner(
    conn: &Connection,
    initialization_id: &str,
    fence: &LaneInitializationFence,
) -> Result<bool>;

pub(crate) fn owner_fence_matches(
    conn: &Connection,
    initialization_id: &str,
    fence: &LaneInitializationFence,
) -> Result<bool>;
```

Every predicate includes token and generation. Heartbeat requires one row;
release deletes only the exact owner.

- [ ] **Step 6: Run GREEN and commit**

```bash
cargo test -p trail --lib db::lane::initialization_owner::tests -- --nocapture
cargo fmt --all -- --check
git add trail/src/db/lane/initialization_owner.rs trail/src/db/lane/mod.rs trail/src/db/lane/initialization.rs
git commit -m "feat: coordinate lane initialization in sqlite"
```

---

### Task 3: Fence durable phases and association

**Files:** `trail/src/db/lane/initialization.rs`, `trail/src/db/lane/lifecycle.rs`, and focused additions to `trail/tests/lane_initialization_faults.rs`.

- [ ] **Step 1: Implement exact-fence phase transitions**

Require `LaneInitializationFence` in every phase API. Add exact token/generation owner predicates to materialized, associated, observer-ready, and repair-required mutations. When a transition is terminal, delete the exact owner in the same transaction and require one deletion. Reread initialization and owner rows to distinguish idempotent replay from ownership loss.

- [ ] **Step 2: Fence association and committed repair**

Before ref/lane/branch/event inserts, require the exact owner in the same `BEGIN IMMEDIATE` transaction. Lost fences roll back all association inserts. Post-association failure sets `repair_required` and removes the owner atomically; persistence failure retains `COMMITTED_REPAIR_REQUIRED` with the actual durable reread phase.

- [ ] **Step 3: Add focused regression coverage after implementation**

Cover stale-owner rejection after generation takeover, terminal transition plus owner release atomicity, and owner-release failure preserving the committed result and actual phase. Extend the existing fixture only as needed; do not build a parallel test-only state machine.

- [ ] **Step 4: Verify and commit**

```bash
cargo test -p trail --test lane_initialization_faults stale_owner_cannot_transition_after_generation_takeover -- --exact --nocapture
cargo test -p trail --test lane_initialization_faults terminal_transition_and_owner_release_are_one_transaction -- --exact --nocapture
cargo test -p trail --test lane_initialization_faults repair_state_persistence_failure_preserves_committed_outcome_contract -- --exact --nocapture
cargo test -p trail --test lane_initialization_faults observer_ready_and_repair_persistence_failures_preserve_committed_outcome_contract -- --exact --nocapture
git add trail/src/db/lane/initialization.rs trail/src/db/lane/lifecycle.rs trail/tests/lane_initialization_faults.rs
git commit -m "fix: fence lane initialization publications"
```

---

### Task 4: Contender wait, crash takeover, and public errors

**Files:** `trail/src/db/lane/initialization_owner.rs`, `trail/src/db/lane/lifecycle.rs`, `trail/src/error.rs`, `trail/src/model/reports/maintenance.rs`, `trail/src/mcp/response.rs`, `trail/tests/lane_initialization_faults.rs`, and `trail/tests/lane_initialization.rs`.

- [ ] **Step 1: Implement wait, replay, and takeover lifecycle**

Use a 10 ms initial delay, 250 ms maximum, bounded jitter, and 30-minute production timeout. Read only initialization/owner rows while waiting. Retry claims for missing or liveness-proven dead/mismatched owners, replay terminal rows, heartbeat around slow work, and exact-release every pre-association early return. Never hold a SQLite write transaction during slow work.

- [ ] **Step 2: Add stable outcomes**

Add `LANE_INITIALIZATION_IN_PROGRESS` as a public exit-status-2 / HTTP-409 / MCP outcome with lane, initialization id, owner PID, phase, and retry guidance. Keep `LANE_INITIALIZATION_OWNERSHIP_LOST` internal: every spawn call site restarts claim/replay instead of exposing it as an ordinary user failure.

- [ ] **Step 3: Add regression coverage after implementation**

Cover 16 identical processes producing one initialization/ref/lane/spawn event and zero terminal owners, live-owner timeout preserving the exact fence, and dead-process takeover/replay at every crash boundary. Tests may inject deterministic short wait policy without changing production constants.

- [ ] **Step 4: Verify and commit**

```bash
cargo test -p trail --test lane_initialization_faults concurrent_identical_processes_replay_one_committed_result -- --exact --nocapture
cargo test -p trail --test lane_initialization_faults live_owner_timeout_is_stable_and_never_revokes -- --exact --nocapture
cargo test -p trail --test lane_initialization_faults dead_process_owner_is_taken_over_and_replays_after_every_crash_cut -- --exact --nocapture
cargo test -p trail --test lane_initialization -- --nocapture
git add trail/src/db/lane/initialization_owner.rs trail/src/db/lane/lifecycle.rs trail/src/error.rs trail/src/model/reports/maintenance.rs trail/src/mcp/response.rs trail/tests/lane_initialization_faults.rs trail/tests/lane_initialization.rs
git commit -m "feat: replay concurrent lane initialization"
```

---

### Task 5: Remove filesystem authority and prove scale/resource behavior

**Files:** `trail/src/db/lane/lifecycle.rs`, `trail/src/db/lane/mod.rs`, `trail/src/db/mod.rs`, `trail/src/lib.rs`, `trail/tests/lane_initialization_faults.rs`, `trail/tests/fixtures/changed_path_raw_mutations.v1`, `docs/reference/cli/lanes.md`, `scripts/check-real-repo-lane-scale.py`, and `scripts/test_check_real_repo_lane_scale.py`.

- [ ] **Step 1: Delete filesystem singleflight**

Remove `LaneInitializationSingleflight`, its platform lock/open/rename helpers, publication hooks/counters/exports, and filesystem-only tests. Do not add compatibility readers or cleanup code because this mechanism never shipped on `main`. Remove only the corresponding reviewed raw-mutation inventory rows.

- [ ] **Step 2: Add owner-count qualification and documentation**

Query and record `active_owner_rows`; require zero after completion and cleanup while preserving existing runtime-resource equality checks. Update CLI documentation for SQLite claim, live-owner protection, liveness-proven takeover, 30-minute in-progress result, and quiescent-copy portability.

- [ ] **Step 3: Add regression and scale coverage after implementation**

Cover absence of lane-initialization filesystem artifacts, 64 unrelated initializations reaching observer-ready with zero active owners, and quiescent workspace-copy replay. Add Python fixtures that reject `active_owner_rows: 1` and accept zero; retain the synthetic `.lock` leak rejection test.

- [ ] **Step 4: Verify and commit**

```bash
cargo test -p trail --test lane_initialization_faults -- --nocapture
cargo test -p trail --test changed_path_ledger_producers -- --nocapture
python3 -m unittest scripts.test_check_real_repo_lane_scale
cargo fmt --all -- --check
cargo clippy -p trail --all-targets -- -D warnings
git add trail/src/db/lane/lifecycle.rs trail/src/db/lane/mod.rs trail/src/db/mod.rs trail/src/lib.rs trail/tests/lane_initialization_faults.rs trail/tests/fixtures/changed_path_raw_mutations.v1 docs/reference/cli/lanes.md scripts/check-real-repo-lane-scale.py scripts/test_check_real_repo_lane_scale.py
git commit -m "refactor: remove lane filesystem singleflight"
```

---

### Task 6: Qualification and merge-readiness evidence

**Files:** modify production/tests only if qualification exposes a real regression; record `.superpowers/sdd/sqlite-lane-coordination-final-report.md` as untracked evidence.

- [ ] **Step 1: Run format, diff, and strict lint**

```bash
cargo fmt --all -- --check
git diff --check main..HEAD
cargo clippy -p trail --all-targets -- -D warnings
```

- [ ] **Step 2: Run focused, serial, and script suites**

```bash
cargo test -p trail --test schema_v20_lane_initialization_owners -- --nocapture
cargo test -p trail --test lane_initialization_faults -- --nocapture
cargo test -p trail --test lane_initialization -- --nocapture
cargo test -p trail --test changed_path_ledger_producers -- --nocapture
cargo test -p trail --lib -- --test-threads=1
python3 -m unittest scripts.test_check_real_repo_lane_scale
python3 -m unittest scripts.test_verify_real_repo_lane_scale
python3 -m unittest scripts.test_verify_real_repo_lane_scale_matrix
```

- [ ] **Step 3: Run the blocking disposable Superset gate**

Use at least 64 COW lanes, multi-file edits, concurrent checkpoint/record, cleanup, Git handoff, and final owner/resource checks. Require zero active owners, duplicate lane/ref/spawn rows, and runtime resource leaks, plus `git_handoff_verified=true`. If the environment blocks the gate, record the exact command, exit status, and prerequisite and do not call it passed.

- [ ] **Step 4: Obtain independent final review**

Package the complete branch diff from the merge base and dispatch a read-only reviewer with the design, plan, reports, and qualification evidence. Resolve every Critical or Important finding, add focused regression coverage after the fix, and re-review.

- [ ] **Step 5: Record final evidence**

Write the untracked final report with SHAs, commands, exit codes, counts, reviewer verdict, Windows limitations, and any unrun scale gates.

---

## Completion conditions

- Schema v20 migration and validation are exact and transactional.
- Same-request thread/process concurrency publishes once and replays.
- Dead-owner takeover is liveness-proven and generation-fenced.
- Live owners are never stolen due to time.
- Every durable phase mutation is owner-fenced.
- Terminal results have zero owner rows.
- No filesystem singleflight artifacts or code remain.
- Focused, strict-lint, serial-library, script, and blocking scale gates have exact evidence.
- Independent review reports no open Critical or Important findings.
- Only then may the feature branch be fast-forwarded to `main` using the preservation-branch/update-ref procedure that leaves the primary dirty Cargo and Prolly state byte-for-byte unchanged.
