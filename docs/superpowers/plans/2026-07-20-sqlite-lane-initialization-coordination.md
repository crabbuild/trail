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
- Use red-green TDD for every production behavior change.
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

**Files:**
- Modify: `trail/src/db/lane/initialization.rs`
- Modify: `trail/src/db/lane/lifecycle.rs`
- Modify: `trail/tests/lane_initialization_faults.rs`

**Interfaces:**
- Consumes Task 2's fence.
- Produces owner-fenced materialized, associated, observer-ready, repair-required, and release transactions.

- [ ] **Step 1: Write RED stale-owner and atomic-release tests**

```rust
#[test]
fn stale_owner_cannot_transition_after_generation_takeover() {
    let fixture = LaneInitializationFixture::new();
    let stale = fixture.claim_owner("fenced-lane");
    fixture.force_dead_owner_takeover("fenced-lane");
    let error = fixture.mark_materialized_with(&stale).unwrap_err();
    assert_eq!(error.code(), "LANE_INITIALIZATION_OWNERSHIP_LOST");
    assert_eq!(fixture.phase("fenced-lane"), LaneInitializationPhase::Reserved);
}

#[test]
fn terminal_transition_and_owner_release_are_one_transaction() {
    let fixture = LaneInitializationFixture::new();
    let fence = fixture.install_associated_owner("terminal-lane");
    fixture.mark_observer_ready_with(&fence).unwrap();
    assert_eq!(fixture.phase("terminal-lane"), LaneInitializationPhase::ObserverReady);
    assert_eq!(fixture.owner_count("terminal-lane"), 0);
}

#[test]
fn owner_release_failure_preserves_committed_result_and_actual_phase() {
    let fixture = LaneInitializationFixture::new();
    let fence = fixture.install_associated_owner("release-failure");
    fixture.reject_owner_delete();
    let error = fixture.mark_observer_ready_with(&fence).unwrap_err();
    assert_eq!(error.code(), "COMMITTED_REPAIR_REQUIRED");
    let Error::CommittedRepairRequired { phase, .. } = error else { panic!() };
    assert_eq!(phase, fixture.phase("release-failure"));
}
```

Extend the integration fixture with these exact methods:

```rust
impl LaneInitializationFixture {
    fn claim_owner(&self, lane: &str) -> LaneInitializationFence;
    fn force_dead_owner_takeover(&self, lane: &str) -> LaneInitializationFence;
    fn mark_materialized_with(&self, fence: &LaneInitializationFence) -> trail::Result<()>;
    fn install_associated_owner(&self, lane: &str) -> LaneInitializationFence;
    fn mark_observer_ready_with(&self, fence: &LaneInitializationFence) -> trail::Result<()>;
    fn phase(&self, lane: &str) -> LaneInitializationPhase;
    fn owner_count(&self, lane: &str) -> i64;
    fn reject_owner_delete(&self);
}
```

- [ ] **Step 2: Run RED**

```bash
cargo test -p trail --test lane_initialization_faults stale_owner_cannot_transition_after_generation_takeover -- --exact --nocapture
cargo test -p trail --test lane_initialization_faults terminal_transition_and_owner_release_are_one_transaction -- --exact --nocapture
```

- [ ] **Step 3: Require fences in phase APIs**

```rust
pub(crate) fn transition_lane_initialization(
    tx: &Transaction<'_>,
    initialization_id: &str,
    fence: &LaneInitializationFence,
    expected: LaneInitializationPhase,
    next: LaneInitializationPhase,
    update: LaneInitializationUpdate<'_>,
    release_owner: bool,
) -> Result<()>;
```

Add an exact-owner `EXISTS` predicate to the phase update. When
`release_owner=true`, delete the exact owner in the same transaction and require
one deletion. Reread both rows to distinguish phase replay from lost ownership.

- [ ] **Step 4: Fence association and committed repair**

Before ref/lane/branch/event inserts, require the owner in the same
`BEGIN IMMEDIATE`. Add this predicate to the associated update:

```sql
AND EXISTS (
  SELECT 1 FROM lane_initialization_owners owner
  WHERE owner.initialization_id=lane_initializations.initialization_id
    AND owner.owner_token=?4 AND owner.owner_generation=?5
)
```

Lost fences roll back all association inserts. Post-association failure sets
`repair_required` and removes the owner atomically; persistence failure retains
`COMMITTED_REPAIR_REQUIRED` with the actual reread phase.

- [ ] **Step 5: Run GREEN and commit**

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

**Files:**
- Modify: `trail/src/db/lane/initialization_owner.rs`
- Modify: `trail/src/db/lane/lifecycle.rs`
- Modify: `trail/src/error.rs`
- Modify: `trail/src/model/reports/maintenance.rs`
- Modify: `trail/src/mcp/response.rs`
- Modify: `trail/tests/lane_initialization_faults.rs`
- Modify: `trail/tests/lane_initialization.rs`

**Interfaces:**
- Produces `wait_for_lane_initialization`, `LaneInitializationInProgress`, and replay/takeover lifecycle.
- Consumes Task 2 claim and Task 3 fenced transitions.

- [ ] **Step 1: Write RED process replay, timeout, and crash tests**

```rust
#[test]
fn concurrent_identical_processes_replay_one_committed_result() {
    let fixture = SharedLaneFixture::new();
    let reports = fixture.spawn_same_request_in_processes(16);
    assert!(reports.iter().all(Result::is_ok));
    assert_eq!(fixture.initialization_count(), 1);
    assert_eq!(fixture.owner_count(), 0);
    assert_eq!(fixture.ref_count(), 1);
    assert_eq!(fixture.lane_count(), 1);
    assert_eq!(fixture.spawn_event_count(), 1);
}

#[test]
fn live_owner_timeout_is_stable_and_never_revokes() {
    let fixture = SharedLaneFixture::new_with_test_wait_policy();
    let owner = fixture.park_live_owner("busy-lane");
    let error = fixture.spawn("busy-lane").unwrap_err();
    assert_eq!(error.code(), "LANE_INITIALIZATION_IN_PROGRESS");
    assert_eq!(fixture.stored_fence("busy-lane"), owner.fence());
}
```

The integration target defines this exact shared-workspace process helper:

```rust
struct SharedLaneFixture { root: tempfile::TempDir, workspace: PathBuf }
impl SharedLaneFixture {
    fn new() -> Self;
    fn new_with_test_wait_policy() -> Self;
    fn spawn(&self, lane: &str) -> trail::Result<LaneSpawnReport>;
    fn spawn_same_request_in_processes(&self, count: usize) -> Vec<trail::Result<LaneSpawnReport>>;
    fn park_live_owner(&self, lane: &str) -> ParkedOwnerChild;
    fn crash_owner_at(&self, boundary: &str);
    fn initialization_count(&self) -> i64;
    fn owner_count(&self) -> i64;
    fn ref_count(&self) -> i64;
    fn lane_count(&self) -> i64;
    fn spawn_event_count(&self) -> i64;
    fn stored_fence(&self, lane: &str) -> LaneInitializationFence;
}
```

- [ ] **Step 2: Run RED**

```bash
cargo test -p trail --test lane_initialization_faults concurrent_identical_processes_replay_one_committed_result -- --exact --nocapture
cargo test -p trail --test lane_initialization_faults live_owner_timeout_is_stable_and_never_revokes -- --exact --nocapture
```

- [ ] **Step 3: Implement the wait policy and contender loop**

```rust
#[derive(Clone, Copy)]
pub(crate) struct LaneInitializationWaitPolicy {
    pub(crate) initial: Duration,
    pub(crate) maximum: Duration,
    pub(crate) timeout: Duration,
}

impl Default for LaneInitializationWaitPolicy {
    fn default() -> Self {
        Self {
            initial: Duration::from_millis(10),
            maximum: Duration::from_millis(250),
            timeout: Duration::from_secs(30 * 60),
        }
    }
}
```

The loop reads only initialization/owner rows, retries claim for missing/dead
owners, and replays terminal state. Tests inject deterministic zero-jitter and
short timeout without changing production constants.

Use these exact wait and replay interfaces:

```rust
pub(crate) enum LaneInitializationWaitOutcome {
    RetryClaim,
    Terminal(LaneInitializationRecord),
}

pub(crate) fn wait_for_lane_initialization(
    db: &Trail,
    request: &ResolvedLaneSpawnRequest,
    policy: LaneInitializationWaitPolicy,
) -> Result<LaneInitializationWaitOutcome>;

fn replay_terminal_lane_initialization(
    record: LaneInitializationRecord,
) -> Result<LaneSpawnReport>;
```

- [ ] **Step 4: Add stable errors**

```rust
LaneInitializationInProgress {
    lane: String,
    initialization_id: String,
    owner_pid: u32,
    phase: LaneInitializationPhase,
    retry: String,
},
LaneInitializationOwnershipLost {
    lane: String,
    initialization_id: String,
    phase: LaneInitializationPhase,
},
```

Codes are `LANE_INITIALIZATION_IN_PROGRESS` and
`LANE_INITIALIZATION_OWNERSHIP_LOST`. Only `LANE_INITIALIZATION_IN_PROGRESS`
is a public CLI/HTTP/MCP outcome (exit status 2, HTTP 409, all fields in JSON
details). `LANE_INITIALIZATION_OWNERSHIP_LOST` is an internal fence signal:
every spawn call site must catch it and restart claim/replay, so it never leaks
as a user-visible ordinary failure.

- [ ] **Step 5: Replace spawn singleflight with claim/wait/replay**

```rust
let (initialization, fence, resumed) = loop {
    match claim_lane_initialization_owner(self, &request)? {
        LaneInitializationClaim::Owned { record, fence, resumed } => {
            break (record, fence, resumed)
        }
        LaneInitializationClaim::Terminal(record) => {
            return replay_terminal_lane_initialization(record)
        }
        LaneInitializationClaim::Contended { .. } => {
            match wait_for_lane_initialization(self, &request, wait_policy)? {
                LaneInitializationWaitOutcome::RetryClaim => continue,
                LaneInitializationWaitOutcome::Terminal(record) => {
                    return replay_terminal_lane_initialization(record)
                }
            }
        }
    }
};
```

Heartbeat before/after slow phases. Exact-release every pre-association early
return. Do not hold a transaction during slow work.

- [ ] **Step 6: Run GREEN and commit**

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

**Files:**
- Modify: `trail/src/db/lane/lifecycle.rs`
- Modify: `trail/src/db/lane/mod.rs`
- Modify: `trail/src/db/mod.rs`
- Modify: `trail/src/lib.rs`
- Modify: `trail/tests/lane_initialization_faults.rs`
- Modify: `trail/tests/fixtures/changed_path_raw_mutations.v1`
- Modify: `docs/reference/cli/lanes.md`
- Modify: `scripts/check-real-repo-lane-scale.py`
- Modify: `scripts/test_check_real_repo_lane_scale.py`

**Interfaces:**
- Consumes complete SQLite coordination from Tasks 1–4.
- Produces zero lane-initialization filesystem resources and owner-row scale evidence.

- [ ] **Step 1: Write RED artifact and scale tests**

```rust
#[test]
fn lane_initialization_coordination_creates_no_filesystem_authority_artifacts() {
    let fixture = SharedLaneFixture::new();
    fixture.spawn("artifact-free").unwrap();
    let names = fixture.trail_resource_names();
    assert!(!names.iter().any(|name| {
        name.contains("lane-initialization-locks")
            || name.contains("lane-initialization-publication")
            || name.ends_with(".anchor")
            || name.ends_with(".identity")
    }));
}

#[test]
fn sixty_four_unrelated_initializations_finish_without_active_owners() {
    let fixture = SharedLaneFixture::new();
    fixture.spawn_distinct_lanes(64).unwrap();
    assert_eq!(fixture.observer_ready_count(), 64);
    assert_eq!(fixture.owner_total(), 0);
}

#[test]
fn quiescent_workspace_copy_replays_without_filesystem_authority() {
    let fixture = SharedLaneFixture::new();
    fixture.spawn("copy-lane").unwrap();
    let copied = fixture.copy_quiescent_workspace();
    let replay = copied.spawn("copy-lane").unwrap();
    assert_eq!(replay.phase, LaneInitializationPhase::ObserverReady);
    assert_eq!(copied.owner_total(), 0);
}
```

Extend `SharedLaneFixture` for this task with:

```rust
impl SharedLaneFixture {
    fn trail_resource_names(&self) -> Vec<String>;
    fn spawn_distinct_lanes(&self, count: usize) -> trail::Result<Vec<LaneSpawnReport>>;
    fn observer_ready_count(&self) -> i64;
    fn owner_total(&self) -> i64;
    fn copy_quiescent_workspace(&self) -> SharedLaneFixture;
}
```

Add Python fixtures where `active_owner_rows: 1` is rejected and zero is
accepted. Retain the synthetic `.lock` leak rejection test.

- [ ] **Step 2: Run RED**

```bash
cargo test -p trail --test lane_initialization_faults lane_initialization_coordination_creates_no_filesystem_authority_artifacts -- --exact --nocapture
python3 -m unittest scripts.test_check_real_repo_lane_scale
```

- [ ] **Step 3: Delete filesystem singleflight**

Remove `LaneInitializationSingleflight`, platform open/lock/rename helpers used
only by it, publication hooks/counters/exports, and filesystem-specific tests.
Do not add compatibility readers or cleanup code because the mechanism never
shipped on `main`. Remove only corresponding reviewed raw-mutation inventory
rows.

- [ ] **Step 4: Add scale owner count and docs**

Query `SELECT COUNT(*) FROM lane_initialization_owners`, record
`active_owner_rows`, require zero after completion/cleanup, and preserve all
existing runtime-resource equality checks. Replace CLI docs with the SQLite
claim, live-owner, takeover, 30-minute in-progress, and quiescent-copy contract.

- [ ] **Step 5: Run GREEN and commit**

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

**Files:**
- Modify only if evidence exposes a regression; every fix needs a RED test and separate commit.
- Record: `.superpowers/sdd/sqlite-lane-coordination-final-report.md` (untracked evidence).

**Interfaces:**
- Consumes Tasks 1–5.
- Produces exact verification evidence and independent merge verdict.

- [ ] **Step 1: Run format, diff, and strict lint**

```bash
cargo fmt --all -- --check
git diff --check main..HEAD
cargo clippy -p trail --all-targets -- -D warnings
```

Expected: all exit zero.

- [ ] **Step 2: Run focused and serial tests**

```bash
cargo test -p trail --test schema_v20_lane_initialization_owners -- --nocapture
cargo test -p trail --test lane_initialization_faults -- --nocapture
cargo test -p trail --test lane_initialization -- --nocapture
cargo test -p trail --test changed_path_ledger_producers -- --nocapture
cargo test -p trail --lib -- --test-threads=1
```

Expected: zero failures. Report native observer/FSEvents transients separately;
an isolated rerun does not make a failed broad run clean.

- [ ] **Step 3: Run script tests**

```bash
python3 -m unittest scripts.test_check_real_repo_lane_scale
python3 -m unittest scripts.test_verify_real_repo_lane_scale
python3 -m unittest scripts.test_verify_real_repo_lane_scale_matrix
```

Expected: all exit zero.

- [ ] **Step 4: Run blocking disposable Superset gate**

Use the existing documented qualification command with at least 64 COW lanes,
multi-file edits, concurrent checkpoint/record, cleanup, Git handoff, and final
owner/resource checks. Required evidence:

```text
active_owner_rows=0
duplicate_lane_rows=0
duplicate_ref_rows=0
duplicate_spawn_events=0
runtime_resource_leaks=0
git_handoff_verified=true
```

If an environmental prerequisite blocks the gate, record exact command, exit
status, and prerequisite; do not call it passed.

- [ ] **Step 5: Obtain independent final review**

```bash
BASE=$(git merge-base main HEAD)
/Users/haipingfu/.codex/plugins/cache/openai-curated-remote/superpowers/6.1.1/skills/subagent-driven-development/scripts/review-package "$BASE" HEAD
```

Dispatch a read-only reviewer with the design, plan, reports, and package. Fix
every Critical/Important finding through a new TDD task and re-review.

- [ ] **Step 6: Record final evidence**

Write `.superpowers/sdd/sqlite-lane-coordination-final-report.md` with SHAs,
commands, exit codes, counts, reviewer verdict, Windows limitations, and unrun
scale gates. Do not commit generated evidence.

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
