# Trail Production Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Trail lane initialization resumable and transactionally honest under large-repository concurrency, close all known correctness and test-isolation gaps, and enforce repeatable production release gates.

**Architecture:** Schema 19 makes a durable `lane_initializations` state machine the authority for lane-spawn identity and recovery, while native-COW materialization remains concurrent outside short authenticated workspace-lock cuts. Versioned role-aware lock admission serializes association and observer publication, journal recovery preserves newer dirty tails, and the CLI/API return stable committed-versus-rolled-back outcomes. Focused fault tests, serial/parallel suites, native-COW accounting, a reproducible prolly pin, and Superset-class scale automation form the release boundary.

**Tech Stack:** Rust, rusqlite/SQLite, serde/serde_json, SHA-256, clap, Trail HTTP/OpenAPI, Git, Bash, Python unittest, macOS APFS native COW, Linux native changed-path runners.

## Global Constraints

- Schema version is exactly `19`; schema 18 upgrades transactionally and schema 19 must be rejected explicitly by older binaries.
- The canonical lane-spawn fingerprint is versioned and includes immutable source change/root, requested mode/destination, normalized sparse paths, neighbor policy, provider, and model.
- Native-COW cloning stays outside the workspace write lock; only reservation, association, and observer-publication authority cuts serialize.
- Live, unverified, malformed, PID-reused, executable-replaced, filesystem-replaced, or policy-incompatible lock owners remain fail-closed; timeouts never revoke a live owner.
- An outcome at or after `associated` is always reported as committed and repairable, never as an absent lane.
- Journal recovery never rotates over or discards a newer dirty tail; a rotation advances exactly one generation at an exact committed cut.
- Native-COW space output must not call APFS allocated blocks exclusive or shared bytes unless authenticated extent evidence proves that claim.
- Do not relax correctness assertions or merely increase latency thresholds to make deterministic or parallel tests pass.
- Preserve unrelated user changes in the CrabDB worktree and `/Volumes/Workspace/Github/superset`.
- The blocking release gate is 64 simultaneous native-COW lanes with at least 50 disjoint edits each; the non-blocking latency stress gate is 128 lanes with identical correctness requirements.

---

### Task 1: Integrate the already-proven scale correctness fixes

**Files:**
- Modify: `trail/src/db/change_ledger/snapshot.rs`
- Modify: `trail/src/db/storage/worktree_index.rs`
- Modify: `trail/src/db/core/workspace/ignore.rs`
- Modify: `trail/src/db/storage/git.rs`
- Modify: `trail/src/db/merge/git_export.rs`
- Modify: `trail/src/db/lane/workdir/record.rs`
- Modify: `trail/src/cli/command/handler/daemon_start.rs`
- Modify: `trail/src/db/change_ledger/mod.rs`
- Modify: `trail/src/db/core/status.rs`
- Modify: `trail/src/db/storage/files.rs`
- Modify: `trail/src/db/storage/worktree_scan.rs`
- Modify: `trail/src/lib.rs`
- Test: `trail/tests/changed_path_ledger_activation.rs`
- Test: `trail/tests/changed_path_ledger_commands.rs`
- Test: `trail/tests/changed_path_ledger_daemon.rs`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Consumes: Git-tracked baseline entries, changed-path candidate snapshots, mapped Git export policy, and daemon startup ownership.
- Produces: tracked ignored paths that remain visible, safe changed-path-only Git handoff, and recovery from a stale daemon process without weakening live-owner checks.

- [ ] **Step 1: Review and split the existing worktree diff by invariant**

Run:

```bash
git diff -- trail/src trail/tests
git diff --check
```

Expected: every source hunk belongs to tracked-ignore preservation, safe Git delta export, or stale-daemon recovery; `Cargo.lock` and the `prolly` gitlink are excluded from this task; `git diff --check` prints nothing.

- [ ] **Step 2: Run the tracked-ignore regressions**

Run:

```bash
cargo test -p trail --test changed_path_ledger_activation tracked_gitignored_file_remains_clean_after_git_import -- --exact --nocapture
cargo test -p trail --test changed_path_ledger_activation trail_full_scan_policy_oracle_matches_nested_untracked_and_ignored_candidates -- --exact --nocapture
cargo test -p trail --test e2e lane_workdir_local_ignore_cannot_hide_materialized_tracked_files -- --exact --nocapture
```

Expected: PASS; ignored untracked files stay excluded, but a baseline path cannot be reported as deleted merely because Git now ignores it.

- [ ] **Step 3: Run Git handoff and daemon recovery regressions**

Run:

```bash
cargo test -p trail --test e2e mapped_git_export -- --nocapture
cargo test -p trail --test changed_path_ledger_commands -- --nocapture
cargo test -p trail --test changed_path_ledger_daemon -- --nocapture
```

Expected: PASS; mapped export contains only the intended changed paths, and a dead daemon publication is recovered while a live authenticated daemon remains authoritative.

- [ ] **Step 4: Commit the qualified correctness fixes without dependency changes**

```bash
git add trail/src trail/tests
git commit -m "fix: preserve large-repository handoff correctness"
```

Expected: one commit containing no `Cargo.lock` or `prolly` gitlink change.

---

### Task 2: Add schema-19 lane initialization storage and migration

**Files:**
- Create: `trail/src/db/lane/initialization.rs`
- Modify: `trail/src/db/lane.rs`
- Modify: `trail/src/db/mod.rs`
- Modify: `trail/src/db/core/init.rs`
- Modify: `trail/src/db/storage/schema.rs`
- Modify: `trail/src/db/storage/schema/ddl.rs`
- Modify: `trail/src/model/reports/lane.rs`
- Modify: `trail/src/lib.rs`
- Test: `trail/src/db/storage/schema/ddl.rs`
- Create: `trail/tests/schema_v19_lane_initializations.rs`

**Interfaces:**
- Consumes: schema-18 `lanes`, `lane_branches`, `lane_events`, materialization metadata, refs, and clean-checkpoint markers.
- Produces: `LaneInitializationPhase`, `LaneInitializationRecord`, `Trail::lane_initialization`, transactional `migrate_schema_v18_to_v19`, and strict schema-19 validation.

- [ ] **Step 1: Write RED schema creation, migration, backfill, rollback, and downgrade tests**

Create `trail/tests/schema_v19_lane_initializations.rs`. Add a `Schema18Fixture` helper that calls the debug-only `trail::test_support::create_schema_v18_fixture`, inserts one fully consistent materialized lane and one lane missing its checkpoint marker, checkpoints WAL, and exposes `workspace() -> &Path`, `db_path() -> &Path`, `database_image_hashes() -> Vec<(String, String)>`, and `clean() -> Self`. Then assert this exact table contract:

```rust
const EXPECTED_COLUMNS: &[&str] = &[
    "initialization_id", "lane_name", "lane_id", "request_fingerprint",
    "operation_id", "phase", "workdir", "materialization_json",
    "last_error_code", "last_error_message", "repair_command",
    "created_at", "updated_at",
];

fn sqlite_user_version(path: &Path) -> i64 {
    rusqlite::Connection::open(path).unwrap()
        .query_row("PRAGMA user_version", [], |row| row.get(0)).unwrap()
}

#[test]
fn schema_18_migrates_once_and_backfills_every_existing_lane() {
    let fixture = Schema18Fixture::with_clean_and_inconsistent_lanes();
    let db = Trail::open(fixture.workspace()).unwrap();
    assert_eq!(sqlite_user_version(fixture.db_path()), 19);
    assert_eq!(db.lane_initialization("clean").unwrap().unwrap().phase,
               LaneInitializationPhase::ObserverReady);
    let repair = db.lane_initialization("missing-marker").unwrap().unwrap();
    assert_eq!(repair.phase, LaneInitializationPhase::RepairRequired);
    assert_eq!(repair.repair_command.as_deref(),
               Some("trail lane repair-initialization missing-marker"));
    drop(db);
    drop(Trail::open(fixture.workspace()).unwrap());
    assert_eq!(sqlite_user_version(fixture.db_path()), 19);
}

#[test]
fn failed_schema_18_migration_is_byte_invariant_and_retriable() {
    let fixture = Schema18Fixture::clean();
    let before = fixture.database_image_hashes();
    trail::test_support::fail_schema_v19_migration_after_ddl(fixture.db_path());
    assert!(Trail::open(fixture.workspace()).is_err());
    assert_eq!(fixture.database_image_hashes(), before);
    trail::test_support::clear_schema_v19_migration_failure(fixture.db_path());
    drop(Trail::open(fixture.workspace()).unwrap());
    assert_eq!(sqlite_user_version(fixture.db_path()), 19);
}
```

Also persist a real schema-18 fixture produced by the schema-18 DDL path, assert the v19 table columns and unique lane-name index, assert invalid phases fail their `CHECK`, verify backup/restore before and after migration, and run the previous binary against a migrated copy expecting `SCHEMA_REINITIALIZE_REQUIRED` with “found version 19”.

- [ ] **Step 2: Run the schema tests and verify RED**

Run:

```bash
cargo test -p trail --test schema_v19_lane_initializations -- --nocapture
```

Expected: compile failure because schema 19 and `lane_initialization` do not exist.

- [ ] **Step 3: Define the schema and phase types**

Add this DDL to `trail/src/db/storage/schema/ddl.rs`, execute it after the preserved schema-18 base DDL for fresh databases, and validate it by comparing `sqlite_master` with an in-memory schema:

```rust
pub(super) const LANE_INITIALIZATIONS_V19: &str = r#"
CREATE TABLE lane_initializations (
    initialization_id TEXT PRIMARY KEY,
    lane_name TEXT NOT NULL UNIQUE,
    lane_id TEXT NOT NULL,
    request_fingerprint TEXT NOT NULL,
    operation_id TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (phase IN
        ('reserved','materialized','associated','observer_ready','repair_required')),
    workdir TEXT,
    materialization_json TEXT,
    last_error_code TEXT,
    last_error_message TEXT,
    repair_command TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE INDEX lane_initializations_phase_updated_idx
    ON lane_initializations(phase, updated_at);
"#;
```

Define the serialized phase in `trail/src/model/reports/lane.rs` and use it from `trail/src/db/lane/initialization.rs`:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LaneInitializationPhase {
    Reserved,
    Materialized,
    Associated,
    ObserverReady,
    RepairRequired,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LaneInitializationReport {
    pub initialization_id: String,
    pub lane_name: String,
    pub lane_id: String,
    pub request_fingerprint: String,
    pub operation_id: String,
    pub phase: LaneInitializationPhase,
    pub workdir: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub repair_command: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LaneInitializationRecord {
    pub initialization_id: String,
    pub lane_name: String,
    pub lane_id: String,
    pub request_fingerprint: String,
    pub operation_id: String,
    pub phase: LaneInitializationPhase,
    pub workdir: Option<PathBuf>,
    pub materialization_json: Option<String>,
    pub last_error_code: Option<String>,
    pub last_error_message: Option<String>,
    pub repair_command: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
}
```

- [ ] **Step 4: Implement transactional fresh creation and 18-to-19 migration**

Set `TRAIL_SCHEMA_VERSION` to `19`; preserve a literal `SCHEMA_V18_VERSION: i64 = 18` for migration validation. Add these exact entry points:

```rust
pub(crate) fn validate_schema_v18_for_migration(conn: &Connection) -> Result<()>;
pub(crate) fn validate_schema_v19(conn: &Connection) -> Result<()>;
pub(crate) fn create_schema_v19(conn: &Connection) -> Result<()>;
pub(crate) fn migrate_schema_v18_to_v19(conn: &mut Connection) -> Result<()>;

#[cfg(any(test, debug_assertions))]
pub(crate) fn create_schema_v18_for_test(conn: &Connection) -> Result<()>;

#[cfg(any(test, debug_assertions))]
pub(crate) fn create_schema_v18_fixture_for_test(workspace: &Path) -> Result<()>;

#[cfg(test)]
pub(crate) enum SchemaV19MigrationBoundary { AfterDdlBeforeUserVersion }

#[cfg(test)]
pub(crate) fn install_schema_v19_migration_failure(
    db_path: &Path, boundary: SchemaV19MigrationBoundary,
);

#[cfg(test)]
pub(crate) fn clear_schema_v19_migration_failure(db_path: &Path);
```

Expose only a `#[doc(hidden)]` debug wrapper in the existing `trail::test_support` module:

```rust
pub fn create_schema_v18_fixture(workspace: &Path) -> Result<(), String> {
    crate::db::create_schema_v18_fixture_for_test(workspace)
        .map_err(|error| error.to_string())
}

pub fn fail_schema_v19_migration_after_ddl(db_path: &Path) {
    crate::db::install_schema_v19_migration_failure(
        db_path, crate::db::SchemaV19MigrationBoundary::AfterDdlBeforeUserVersion,
    );
}

pub fn clear_schema_v19_migration_failure(db_path: &Path) {
    crate::db::clear_schema_v19_migration_failure(db_path);
}
```

`migrate_schema_v18_to_v19` must use `TransactionBehavior::Immediate`, validate the complete v18 shape before mutation, execute `LANE_INITIALIZATIONS_V19`, call `backfill_lane_initializations_v19(&tx)`, update `schema_meta['schema.version']` and `app.version`, set `PRAGMA user_version = 19`, validate the v19 shape inside the transaction, and commit. Opening version `> 19` or a partial version 18/19 shape returns `SchemaReinitializeRequired` without mutation.

- [ ] **Step 5: Implement conservative existing-lane backfill**

Add:

```rust
pub(super) fn backfill_lane_initializations_v19(tx: &Transaction<'_>) -> Result<()>;
fn backfilled_lane_phase(tx: &Transaction<'_>, lane_id: &str, workdir: Option<&str>)
    -> Result<LaneInitializationPhase>;

pub fn lane_initialization(&self, lane: &str)
    -> Result<Option<LaneInitializationReport>>;
```

For each active `lane_branches` row, derive a versioned legacy fingerprint from lane name, base change/root, stored workdir mode/path, sparse metadata, provider, and model. Mark `observer_ready` only when the ref and branch head match, materialization metadata is complete when a workdir exists, the clean marker/observer scope is consistent, and a `lane_spawned` event exists. Otherwise store `repair_required`, `LANE_INITIALIZATION_INCOMPLETE`, a precise failed invariant, and `trail lane repair-initialization <lane>`.

- [ ] **Step 6: Run schema tests and the existing hard-cutover suite**

Run:

```bash
cargo test -p trail --test schema_v19_lane_initializations -- --nocapture
cargo test -p trail --test schema_v18_hard_cutover -- --nocapture
```

Expected: PASS; the old suite now explicitly treats 18 as the only migratable predecessor and still rejects every corrupt/partial shape byte-invariantly.

- [ ] **Step 7: Commit schema 19 independently**

```bash
git add trail/src/db trail/tests/schema_v19_lane_initializations.rs trail/tests/schema_v18_hard_cutover.rs
git commit -m "feat: add durable lane initialization schema"
```

---

### Task 3: Canonicalize spawn identity and reserve idempotently

**Files:**
- Modify: `trail/src/db/lane/initialization.rs`
- Modify: `trail/src/db/lane/lifecycle.rs`
- Modify: `trail/src/error.rs`
- Modify: `trail/src/cli/command/handler/errors.rs`
- Modify: `trail/src/model/reports/lane.rs`
- Test: `trail/tests/lane_initialization.rs`

**Interfaces:**
- Consumes: resolved immutable `ChangeId`/`ObjectId`, `LaneWorkdirMode`, destination path, normalized sparse paths, neighbor flag, provider, and model.
- Produces: `CanonicalLaneSpawnRequestV1::fingerprint`, `LaneInitializationReservation`, stable `LANE_INITIALIZATION_CONFLICT`, and report identity fields.

- [ ] **Step 1: Write RED canonicalization and duplicate-request tests**

Create `trail/tests/lane_initialization.rs`. Its `LaneInitializationFixture` helper initializes an empty temporary workspace, writes `a/file.txt` and `b/file.txt`, records them on `main`, retains the temporary directory, and exposes `db_mut() -> &mut Trail`. Cover reordered/duplicated sparse paths, relative versus canonical destination, `main` versus `refs/heads/main`, same-fingerprint replay, different-source conflict, different-mode conflict, and different-neighbor/provider/model conflicts:

```rust
#[test]
fn equivalent_spawn_requests_share_one_initialization_identity() {
    let mut fixture = LaneInitializationFixture::new();
    let first = fixture.db_mut().spawn_lane_with_workdir_mode_paths_and_neighbors(
        "agent-1", Some("main"), LaneWorkdirMode::Sparse, Some("codex".into()),
        Some("gpt-5".into()), None, &["b".into(), "a".into(), "a".into()], true,
    ).unwrap();
    let replay = fixture.db_mut().spawn_lane_with_workdir_mode_paths_and_neighbors(
        "agent-1", Some("refs/heads/main"), LaneWorkdirMode::Sparse,
        Some("codex".into()), Some("gpt-5".into()), None,
        &["a".into(), "b".into()], true,
    ).unwrap();
    assert_eq!(replay.initialization_id, first.initialization_id);
    assert_eq!(replay.request_fingerprint, first.request_fingerprint);
    assert!(replay.resumed);
}
```

- [ ] **Step 2: Run the focused tests and verify RED**

Run:

```bash
cargo test -p trail --test lane_initialization canonical -- --nocapture
cargo test -p trail --test lane_initialization conflict -- --nocapture
```

Expected: compile failure because canonical request and report fields are absent.

- [ ] **Step 3: Implement the versioned canonical request**

Add the exact serializable form and use JSON only after sorting/deduplicating paths and normalizing an explicit destination with Trail’s existing safe-path resolver:

```rust
#[derive(Serialize)]
struct CanonicalLaneSpawnRequestV1<'a> {
    version: u8,
    workspace_id: &'a str,
    lane_name: &'a str,
    source_ref: &'a str,
    source_change: &'a str,
    source_root: &'a str,
    requested_workdir_mode: &'a LaneWorkdirMode,
    workdir: Option<&'a str>,
    sparse_paths: &'a [String],
    include_neighbors: bool,
    provider: Option<&'a str>,
    model: Option<&'a str>,
}

impl CanonicalLaneSpawnRequestV1<'_> {
    fn fingerprint(&self) -> Result<String> {
        let bytes = serde_json::to_vec(self)?;
        Ok(format!("sha256:{}", hex::encode(Sha256::digest(bytes))))
    }
}
```

Derive `initialization_id` as SHA-256 over `trail-lane-initialization-v1\0`, workspace ID, lane name, and fingerprint; do not derive it from wall clock or process identity.

Resolve `main` and `refs/heads/main` to the same canonical `source_ref`; a raw change ID uses `detached:<change-id>`. Define the internal request once and reuse it through reservation and every later phase:

```rust
pub(crate) struct ResolvedLaneSpawnRequest {
    pub lane_name: String,
    pub lane_id: String,
    pub source_ref: String,
    pub source_change: ChangeId,
    pub source_root: ObjectId,
    pub source_operation: ObjectId,
    pub requested_workdir_mode: LaneWorkdirMode,
    pub workdir: Option<PathBuf>,
    pub sparse_paths: Vec<String>,
    pub include_neighbors: bool,
    pub provider: Option<String>,
    pub model: Option<String>,
    pub request_fingerprint: String,
    pub initialization_id: String,
}
```

- [ ] **Step 4: Implement reservation and stable conflict**

Add:

```rust
pub(crate) enum LaneInitializationReservation {
    Start(LaneInitializationRecord),
    Resume(LaneInitializationRecord),
    Ready(LaneInitializationRecord),
}

pub(crate) fn reserve_lane_initialization(
    &mut self,
    request: &ResolvedLaneSpawnRequest,
) -> Result<LaneInitializationReservation>;
```

The `IMMEDIATE` reservation transaction inserts `reserved` only when neither a row nor lane ref exists. An equal fingerprint returns `Resume` or `Ready`; a different fingerprint returns:

```rust
Error::LaneInitializationConflict {
    lane: lane_name.to_owned(),
    existing_fingerprint,
    requested_fingerprint,
}
```

Map it to code `LANE_INITIALIZATION_CONFLICT`, exit code `2`, and JSON details containing all three fields. The conflict path must perform zero writes.

At reservation, set `operation_id` to `ResolvedLaneSpawnRequest::source_operation`. When materialization succeeds, the `reserved -> materialized` compare-and-set replaces it with `MaterializationOutcome::materialization_operation_id`; virtual lanes retain the source operation ID.

- [ ] **Step 5: Extend `LaneSpawnReport` without breaking readers**

Add:

```rust
pub initialization_id: String,
pub request_fingerprint: String,
pub phase: LaneInitializationPhase,
pub committed: bool,
pub resumed: bool,
```

Populate all internal constructors; `committed` is false only before association and true for `Associated`, `ObserverReady`, and `RepairRequired` reports.

- [ ] **Step 6: Verify the identity and structured-error contract**

Run:

```bash
cargo test -p trail --test lane_initialization -- --nocapture
cargo test -p trail error::tests --lib -- --nocapture
```

Expected: PASS; equivalent requests return the same identity and conflicts are stable and mutation-free.

- [ ] **Step 7: Commit request identity**

```bash
git add trail/src/db/lane trail/src/error.rs trail/src/cli/command/handler/errors.rs trail/src/model/reports/lane.rs trail/tests/lane_initialization.rs
git commit -m "feat: reserve lane spawns idempotently"
```

---

### Task 4: Drive spawn and repair through durable phases

**Files:**
- Modify: `trail/src/db/lane/initialization.rs`
- Modify: `trail/src/db/lane/lifecycle.rs`
- Modify: `trail/src/cli/command/lane_args.rs`
- Modify: `trail/src/cli/command/handler/lane.rs`
- Modify: `trail/src/cli/command/handler/daemon_rpc.rs`
- Modify: `trail/src/cli/command/render/lane/identity/basic.rs`
- Modify: `trail/src/server/route/lane/lanes.rs`
- Modify: `trail/src/server/openapi/paths/lanes.rs`
- Modify: `trail/src/server/openapi/schemas/lane.rs`
- Test: `trail/tests/lane_initialization.rs`
- Create: `trail/tests/lane_initialization_faults.rs`

**Interfaces:**
- Consumes: `LaneInitializationReservation` and existing staged materialization/association journals.
- Produces: compare-and-set phase transitions, idempotent `repair_lane_initialization`, `trail lane repair-initialization`, `POST /v1/lanes/{lane}/repair-initialization`, and honest committed repair responses.

- [ ] **Step 1: Write RED phase and response-loss fault tests**

In `trail/tests/lane_initialization_faults.rs`, use subprocess failpoints at `after_reservation`, `after_materialization`, `after_association`, `after_reconciliation`, `after_marker`, and `after_spawn_event`. Kill the child after an fsynced handshake, reopen, rerun the identical spawn, and assert:

```rust
assert_eq!(rows_for_lane(&db, lane), 1);
assert_eq!(refs_for_lane(&db, lane), 1);
assert_eq!(spawn_events_for_lane(&db, lane), 1);
assert_eq!(result.phase, LaneInitializationPhase::ObserverReady);
assert!(result.resumed);
```

Add SQL helpers `rows_for_lane`, `refs_for_lane`, and `spawn_events_for_lane` that each run a parameterized `SELECT COUNT(*)` against `lane_initializations`, `refs`, and `lane_events` respectively and return `i64`.

Also inject daemon loss after association and response loss after readiness; the former must return `COMMITTED_REPAIR_REQUIRED` with `committed=true`, while the latter must replay the original successful report.

Add thread-scoped I/O failpoints at workdir write, file sync, directory sync, and association SQLite commit. Return `std::io::Error::from_raw_os_error(28)` for disk-full simulation and `PermissionDenied` for permission simulation. For every boundary, assert the stored phase never advances beyond the durable artifact, pre-association artifacts proven unassociated are cleaned, and associated artifacts remain repairable.

- [ ] **Step 2: Run the fault tests and verify RED**

Run:

```bash
cargo test -p trail --test lane_initialization_faults -- --nocapture
```

Expected: FAIL because retries currently hit “lane already exists” and deferred observer failure can escape as `DAEMON_UNAVAILABLE`.

- [ ] **Step 3: Add compare-and-set phase operations**

Implement these methods so each update shares the transaction that makes its named artifact durable:

```rust
fn transition_lane_initialization(
    tx: &Transaction<'_>, initialization_id: &str,
    expected: LaneInitializationPhase, next: LaneInitializationPhase,
    update: LaneInitializationUpdate<'_>,
) -> Result<()>;

struct LaneInitializationUpdate<'a> {
    operation_id: Option<&'a str>,
    workdir: Option<&'a Path>,
    materialization_json: Option<&'a str>,
    last_error: Option<&'a Error>,
}

fn mark_lane_initialization_repair_required(
    &mut self, initialization_id: &str, error: &Error,
) -> Result<LaneInitializationRecord>;
```

Require exactly one affected row; zero rows means reload and accept only the already-completed identical transition, otherwise return corruption. Persist `last_error_code`, bounded error text, and exactly `trail lane repair-initialization <lane>`.

- [ ] **Step 4: Refactor spawn into resumable phase handlers**

Make the lifecycle explicit:

```rust
fn materialize_reserved_lane(&mut self, init: &LaneInitializationRecord,
    request: &ResolvedLaneSpawnRequest) -> Result<LaneInitializationRecord>;
fn associate_materialized_lane(&mut self, init: &LaneInitializationRecord,
    request: &ResolvedLaneSpawnRequest) -> Result<LaneInitializationRecord>;
fn prepare_associated_lane_observer(&mut self,
    init: &LaneInitializationRecord) -> Result<LaneInitializationRecord>;
fn lane_spawn_report(&self, init: &LaneInitializationRecord,
    resumed: bool) -> Result<LaneSpawnReport>;
```

Large cloning runs before acquiring the association lock. The association transaction commits the ref, `lanes`, `lane_branches`, ref-mirror repair, materialization completion, and `associated` transition together. Reconciliation, marker publication, and a deduplicated `lane_spawned` event then advance to `observer_ready`. Any error after association is durably converted to `repair_required` before returning `CommittedRepairRequired`; remove the CLI-only deferred-resume path that currently loses this distinction. If reservation or materialization fails before association, return an ordinary error and clean up only stage/workdir artifacts whose durable materialization journal proves they are unassociated. Never remove an artifact once an `associated` compare-and-set succeeds.

- [ ] **Step 5: Implement idempotent repair validation**

Add:

```rust
pub fn repair_lane_initialization(&mut self, lane: &str) -> Result<LaneSpawnReport>;
```

It reloads the row under association admission, validates initialization ID/fingerprint, ref and branch identity, materialization journal, no-follow filesystem identity, and current lane head. It resumes only missing reconciliation/marker/event work. It must never clone or copy the workdir when phase is `associated` or `repair_required`; a pre-association materialization may be recovered only through its existing durable materialization journal.

A same-fingerprint spawn in `repair_required` invokes this repair path exactly once during that command. If validation or repair still fails, it updates the same row’s bounded error details and returns committed repair again; it does not recursively retry.

Extend the error variant so JSON does not infer committed state from text:

```rust
Error::CommittedRepairRequired {
    lane: String,
    initialization_id: String,
    request_fingerprint: String,
    operation_id: String,
    phase: LaneInitializationPhase,
    committed: bool,
    repair: String,
    reason: String,
}
```

Update lane removal so its successful transaction deletes the matching `lane_initializations` row only after the lane ref/branch and materialization are durably retired. Add a regression that removes an observer-ready lane, confirms the row is gone, and safely reuses the lane name with a new fingerprint; a failed removal leaves the original initialization untouched.

- [ ] **Step 6: Add CLI, daemon RPC, HTTP, OpenAPI, and render contracts**

Add `LaneSubcommand::RepairInitialization(LaneRepairInitializationArgs { name: String })`, route it locally and through daemon RPC, and expose `POST /v1/lanes/{lane_or_id}/repair-initialization` with no body. Human output must say either “Lane initialized” or “Lane committed; repair required” and show the exact command. JSON uses the extended `LaneSpawnReport` or stable structured error fields; it must never print “spawn failed” for `committed=true`. HTTP spawn returns `201 Created` for the first completion, `200 OK` for an idempotent replay, and `409 Conflict` with committed-repair details when repair remains required.

- [ ] **Step 7: Verify lifecycle, HTTP, and CLI behavior**

Run:

```bash
cargo test -p trail --test lane_initialization -- --nocapture
cargo test -p trail --test lane_initialization_faults -- --nocapture
cargo test -p trail --test e2e repair_initialization -- --nocapture
cargo test -p trail server::openapi --lib -- --nocapture
```

Expected: PASS at every crash cut, exactly one lane/event, stable JSON, and idempotent repair.

- [ ] **Step 8: Commit the state-machine cutover**

```bash
git add trail/src trail/tests/lane_initialization.rs trail/tests/lane_initialization_faults.rs trail/tests/e2e.rs
git commit -m "feat: resume lane initialization across failures"
```

---

### Task 5: Add versioned role-aware workspace-lock admission

**Files:**
- Modify: `trail/src/db/mod.rs`
- Modify: `trail/src/db/core/init.rs`
- Modify: `trail/src/db/change_ledger/log/writer.rs`
- Modify: `trail/src/db/change_ledger/projection.rs`
- Modify: `trail/src/db/lane/initialization.rs`
- Test: `trail/src/db/mod.rs`
- Test: `trail/src/db/change_ledger/log/tests.rs`
- Test: `trail/tests/lane_initialization_faults.rs`

**Interfaces:**
- Consumes: authenticated v1 lock records and current process-start/nonce validation.
- Produces: v2 `WorkspaceLockOwner`, `WorkspaceLockPurpose`, bounded `WorkspaceLockAdmission`, in-process condition notification, cross-process authenticated backoff, and observer-startup admission.

- [ ] **Step 1: Write RED lock record, compatibility, timeout, and concurrency tests**

Cover v1 parse/reap, v2 round trip, malformed/unknown purpose, live PID reuse mismatch, executable identity replacement, file replacement, timeout diagnostics, same-scope observer exclusion, and 64 distinct observer startups. The timeout assertion must include purpose, age, initialization ID, and retry command.

- [ ] **Step 2: Run focused lock tests and verify RED**

Run:

```bash
cargo test -p trail db::tests::workspace_lock --lib -- --nocapture
cargo test -p trail db::change_ledger::log::tests::observer --lib -- --nocapture
```

Expected: FAIL because v1 records lack purpose and observer startup currently performs a single immediate lock attempt.

- [ ] **Step 3: Define the v2 record and admission request**

Add:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WorkspaceLockPurpose {
    CommandMutation, LaneAssociation, ObserverStartup,
    ObserverPublication, SchemaTransition, Maintenance,
}

pub(crate) struct WorkspaceLockAdmission<'a> {
    pub purpose: WorkspaceLockPurpose,
    pub operation_id: Option<&'a str>,
    pub deadline: Duration,
    pub retry_command: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct WorkspaceLockOwner {
    version: u8,
    pid: u32,
    process_start_identity: String,
    executable_identity: String,
    nonce: String,
    purpose: WorkspaceLockPurpose,
    operation_id: Option<String>,
    created_at: i64,
}
```

Encode new owners as `trail-workspace-lock-v2`; parse v1 only to authenticate/reap it. A live v1 or unknown record is incompatible and terminal, never silently admitted.

Compute `executable_identity` from the no-follow metadata of `std::env::current_exe()` as SHA-256 over canonical path, device/inode where available, length, and modification timestamp. Re-read it while authenticating a live contender; replacement or unreadable metadata is terminal.

- [ ] **Step 4: Implement bounded role-aware acquisition**

Add:

```rust
pub(crate) fn acquire_workspace_lock_with_admission(
    db_dir: &Path, schema_path: &Path,
    admission: WorkspaceLockAdmission<'_>,
) -> Result<WorkspaceLock>;
```

Authenticate the held inode and process on every retry. Allow bounded waiting for `ObserverStartup` behind `LaneAssociation`, `ObserverPublication`, or another distinct-scope `ObserverStartup`; allow `LaneAssociation` behind ordinary command/observer publication. Treat `SchemaTransition`, `Maintenance`, v1-live, malformed, replaced, and unverified owners as terminal. Use the existing condition variable for same-process release and 2ms-to-50ms exponential backoff plus nonce-derived jitter cross-process. Read `TRAIL_WORKSPACE_LOCK_WAIT_MS`, default to 30,000ms, reject non-numeric values, and clamp accepted values to 120,000ms. Never unlink a live owner on timeout.

Add a structured timeout distinct from an incompatible immediate lock failure:

```rust
Error::WorkspaceLockTimeout {
    holder_purpose: String,
    holder_age_ms: u64,
    operation_id: Option<String>,
    retry_command: String,
}
```

Map it to `WORKSPACE_LOCK_TIMEOUT` and exit code `8`.

- [ ] **Step 5: Route observer segment publication through admission**

Replace the direct call in `SegmentWriter::acquire_inner` with `acquire_workspace_lock_with_admission` using purpose `ObserverStartup`, the scope/initialization operation ID, and retry command `trail lane repair-initialization <lane>` when available. Keep same-scope segment owner/epoch replacement exclusive; allow expensive distinct-scope scans before the short publication lock.

- [ ] **Step 6: Verify 64 simultaneous initialization admissions**

Run:

```bash
cargo test -p trail --test lane_initialization_faults sixty_four_observers_serialize_publication_without_ambiguous_failures -- --exact --nocapture
cargo test -p trail db::tests::workspace_lock --lib -- --nocapture
```

Expected: PASS; all 64 reports reach `observer_ready`, no `DAEMON_UNAVAILABLE`, and no live or candidate lock remains.

- [ ] **Step 7: Commit lock admission**

```bash
git add trail/src/db trail/tests/lane_initialization_faults.rs
git commit -m "fix: admit concurrent lane observers safely"
```

---

### Task 6: Preserve dirty journal tails during checkpoint recovery

**Files:**
- Modify: `trail/src/db/lane/workdir/view_journal.rs`
- Modify: `trail/src/db/lane/workspace_view.rs`
- Test: `trail/src/db/lane/workdir/view_journal.rs`
- Test: `trail/src/db/lane/workspace_view.rs`

**Interfaces:**
- Consumes: authenticated active journal generation, base sequence/hash, last sequence, SQLite checkpoint generation/sequence/root, mirror, and barrier.
- Produces: `ViewJournalRecoveryState` and recovery that either repairs a marker at the clean cut, performs one exact rotation, accepts an already-rotated cut, or fails closed.

- [ ] **Step 1: Write RED crash-cut and dirty-tail tests**

Add cases for initial `(generation=0, checkpoint_seq=0)`, a same-generation newer source tail, generated/private dirty paths, repeated reopen, crash after SQLite publication, crash after mirror publication, crash after rotation, skipped generation, contradictory base hash, and two views sharing one generated layer. In the dirty-tail test assert byte-for-byte journal equality and unchanged `last_sequence` before/after reopen.

- [ ] **Step 2: Run the journal tests and verify RED**

Run:

```bash
cargo test -p trail db::lane::workspace_view::tests::checkpoint_marker_recovery_preserves_newer_uncheckpointed_source_edits --lib -- --exact --nocapture
cargo test -p trail db::lane::workspace_view::tests --lib -- --nocapture
```

Expected: FAIL with `cannot rotate workspace journal generation ... at sequence ...` for the known cases.

- [ ] **Step 3: Expose authenticated recovery state**

Add:

```rust
pub(crate) struct ViewJournalRecoveryState {
    pub generation: u64,
    pub base_sequence: u64,
    pub last_sequence: u64,
    pub mutation_base_hash: String,
    pub whiteout_base_hash: String,
    pub recovery_qualified: bool,
}

pub(crate) fn recovery_state(upperdir: &Path) -> Result<ViewJournalRecoveryState>;
```

Populate it only after validating state, both journal files, and tail anchor identity. Do not infer hashes from an unauthenticated mirror.

- [ ] **Step 4: Implement the exact recovery decision table**

In `recover_workspace_checkpoint_markers`:

```rust
match (journal.generation.cmp(&generation), journal.last_sequence.cmp(&checkpoint_seq)) {
    (Ordering::Equal, Ordering::Equal | Ordering::Greater) => {
        // Repair marker and barrier at SQLite's clean cut; preserve active journal.
    }
    (Ordering::Less, Ordering::Equal)
        if generation == journal.generation + 1 => {
        ViewMutationJournal::rotate_after_checkpoint(source_upper, checkpoint_seq, generation)?;
    }
    (Ordering::Greater, _)
        if journal.generation == generation + 1
            && journal.base_sequence == checkpoint_seq => {
        // Accept crash-after-rotation only after base hashes match the prior cut.
    }
    _ => return Err(Error::Corrupt(format!(
        "workspace checkpoint/journal cut is contradictory for `{view_id}`"
    ))),
}
```

Write the clean marker atomically, then record the barrier. Rotation is forbidden when the active generation already equals SQLite, including the zero-sequence initial view.

- [ ] **Step 5: Run all workspace journal/view tests**

Run:

```bash
cargo test -p trail db::lane::workdir::view_journal::tests --lib -- --nocapture
cargo test -p trail db::lane::workspace_view::tests --lib -- --nocapture
```

Expected: PASS; repeated recovery is idempotent and dirty tails survive exactly.

- [ ] **Step 6: Commit journal recovery**

```bash
git add trail/src/db/lane/workdir/view_journal.rs trail/src/db/lane/workspace_view.rs
git commit -m "fix: preserve dirty workspace journal tails"
```

---

### Task 7: Fix every deterministic library failure at its root cause

**Files:**
- Modify: `trail/src/db/change_ledger/activation.rs`
- Modify: `trail/tests/fixtures/changed_path_producers.v1`
- Modify: `trail/tests/changed_path_ledger_activation.rs`
- Modify: `trail/src/db/change_ledger/policy.rs`
- Modify: `trail/src/db/change_ledger/daemon.rs`
- Modify: `trail/src/db/change_ledger/recovery.rs`
- Modify: `trail/src/db/core/init.rs`
- Modify: `trail/src/db/mod.rs`
- Modify: `docs/design/native-agent-hooks-and-acp.md`

**Interfaces:**
- Consumes: checked producer fixture, Git environment selector capture, structured reconciliation errors, crash-helper subprocess output, and schema-validation singleflight cache.
- Produces: a verified producer digest, correct empty-selector semantics, preserved reconciliation errors, actionable crash diagnostics, and sub-5-second 100-open behavior with exactly one authority validation.

- [ ] **Step 1: Capture each known deterministic RED independently**

Run with `RUST_TEST_THREADS=1`:

```bash
cargo test -p trail changed_path_producer_inventory_matches_reviewed_fixture -- --exact --nocapture
cargo test -p trail db::change_ledger::policy::tests::missing_and_empty_legacy_git_config_selectors_are_persisted --lib -- --exact --nocapture
cargo test -p trail db::change_ledger::daemon::tests::persistent_startup_policy_churn_exhausts_the_bound_and_fails_closed --lib -- --exact --nocapture
cargo test -p trail db::change_ledger::recovery::harness::subprocess_kill_and_reopen_covers_intent_durability_boundaries --lib -- --exact --nocapture
cargo test -p trail db::core::init::schema_handoff_tests::one_hundred_ordinary_opens_share_one_unchanged_generation_validation --lib -- --exact --nocapture
```

Expected: reproduce the stale digest, unsafe empty `GIT_CONFIG`, generic `DAEMON_UNAVAILABLE`, pre-handshake crash, and 100-open latency failure respectively.

- [ ] **Step 2: Regenerate and verify the activation digest**

First run the producer inventory generator/audit and require it to produce the checked fixture unchanged except for reviewed producer additions. Compute:

```bash
shasum -a 256 trail/tests/fixtures/changed_path_producers.v1
```

Expected digest: `a13fa0330d89ad442a4f796a5fd37b55177ab4fdf7805354925b99fc18199d0e`. Set `APPROVED_PRODUCER_INVENTORY_SHA256` to that exact value only after the audit test passes, then update the approved activation evidence in `docs/design/native-agent-hooks-and-acp.md`.

- [ ] **Step 3: Separate empty selector identity from file dependencies**

In policy capture, retain `GIT_CONFIG=` in the canonical environment-selector vector but gate file dependency insertion:

```rust
if let Some(selected) = git_environment_value(context, "GIT_CONFIG") {
    selectors.push(("GIT_CONFIG".into(), selected.clone()));
    if !selected.is_empty() {
        dependencies.push(resolve_git_config_dependency(context, &selected)?);
    }
}
```

An absent selector and an explicitly empty selector remain distinguishable; neither maps an empty path to the workspace root.

- [ ] **Step 4: Preserve terminal reconciliation errors through daemon startup**

When bounded startup retries exhaust on policy churn, return the last `ChangeLedgerReconcileRequired` unchanged. Wrap only transport/process startup failures as `DaemonUnavailable`; do not discard `scope`, `state`, `reason`, or `command`.

- [ ] **Step 5: Preserve bounded crash-helper diagnostics and fix the phase failure**

Pipe stderr instead of discarding it, drain it on handshake timeout/early exit, cap retained text at 64 KiB, and include phase plus child status in `wait_for_crash_handshake`. In `run_real_crash_scenario`, retain the acquired `SegmentWriter` through `mark_filesystem_applied`: the proof authority query requires the observer owner lease to remain active, but the helper currently drops `writer` before validating the proof. Move `drop(writer)` to immediately after `mark_filesystem_applied` returns; the helper then fsyncs its ready file only after the requested durable boundary is reached.

- [ ] **Step 6: Profile and optimize 100 ordinary opens**

Keep the assertions `validation_count == 1` and elapsed `< 5 seconds`. Remove repeated immutable generation-key hashing/stat work from followers by caching the validated generation tuple with the existing singleflight result; invalidate only on authoritative DB/WAL generation change. Do not increase five seconds or skip schema validation.

- [ ] **Step 7: Verify all deterministic fixes together**

Run the five commands from Step 1 plus:

```bash
RUST_TEST_THREADS=1 cargo test -p trail --lib
```

Expected: all five focused tests PASS and the serial library suite has zero failures.

- [ ] **Step 8: Commit deterministic repairs**

```bash
git add trail/src/db trail/tests/changed_path_ledger_activation.rs trail/tests/fixtures/changed_path_producers.v1 docs/design/native-agent-hooks-and-acp.md
git commit -m "fix: close deterministic production test gaps"
```

---

### Task 8: Eliminate parallel-only shared-state interference

**Files:**
- Modify: `trail/src/acp/capture.rs`
- Modify: `trail/src/db/core/init.rs`
- Modify: `trail/src/db/mod.rs`
- Modify: `trail/src/db/lane/control/agent_capture.rs`
- Create: `trail/src/test_support/scoped_state.rs`
- Modify: `trail/src/lib.rs`

**Interfaces:**
- Consumes: process-global failpoint maps, schema server registry, runtime socket/lock paths, environment overrides, ACP shutdown hooks, and duplicate-receipt test workspaces.
- Produces: workspace/test-keyed scoped state with RAII cleanup and five tests that pass both alone and under default parallelism.

- [ ] **Step 1: Reproduce and inventory only parallel failures**

Run each test alone, then the full suite with default threads and `--nocapture`. Record which global registry, path, hook, or environment key each failing test touches:

```bash
cargo test -p trail acp::capture::tests::queue_overflow_spills_every_frame_and_shutdown_is_bounded --lib -- --exact --nocapture
cargo test -p trail db::core::init::schema_handoff_tests --lib -- --nocapture
cargo test -p trail db::lane::control::agent_capture::tests::one_hundred_concurrent_duplicate_receipts_create_one_journal_row --lib -- --exact --nocapture
cargo test -p trail --lib -- --nocapture
```

Expected: the focused tests pass alone; the default parallel suite exposes shared-state interference before the fix.

- [ ] **Step 2: Add a scoped-state guard**

Implement:

```rust
pub(crate) struct ScopedTestState<K: Eq + Hash + Clone + 'static, V: 'static> {
    key: K,
    map: &'static Mutex<HashMap<K, V>>,
}

impl<K: Eq + Hash + Clone + 'static, V: 'static> ScopedTestState<K, V> {
    pub(crate) fn install(
        map: &'static Mutex<HashMap<K, V>>, key: K, value: V,
    ) -> Self {
        let previous = map.lock().unwrap_or_else(|p| p.into_inner())
            .insert(key.clone(), value);
        assert!(previous.is_none(), "scoped test key was already installed");
        Self { key, map }
    }
}

impl<K: Eq + Hash + Clone + 'static, V: 'static> Drop for ScopedTestState<K, V> {
    fn drop(&mut self) {
        self.map.lock().unwrap_or_else(|p| p.into_inner()).remove(&self.key);
    }
}
```

Inside the existing debug-only `test_support` module in `trail/src/lib.rs`, declare `#[path = "scoped_state.rs"] pub(crate) mod scoped_state;` so the new file resolves as `trail/src/test_support/scoped_state.rs` without moving the existing public test-support API.

Use canonical temp workspace path plus a unique test nonce as the key. Environment overrides must use the existing serialized scoped-environment helper and restore the prior value on drop. Runtime sockets/locks must live below each fixture’s `.trail/tmp/tests/<nonce>` rather than a shared process path. The specific isolated regressions are `queue_overflow_spills_every_frame_and_shutdown_is_bounded`, `production_server_start_does_not_rendezvous_with_the_spawned_thread`, `generation_replacement_does_not_wait_for_the_old_server`, `schema_validation_leader_failure_propagates_and_next_open_retries`, and `one_hundred_concurrent_duplicate_receipts_create_one_journal_row`.

- [ ] **Step 3: Key schema and ACP hooks by fixture identity**

Replace singleton failpoint/hook slots with keyed maps. Schema generation replacement, production start, leader failure, and ACP bounded shutdown must consume only the hook installed for their workspace/test nonce. Ensure panic paths drop the guard and wake waiters.

- [ ] **Step 4: Isolate duplicate-receipt timing and authority**

Give the 100-receipt test its own workspace lock/runtime namespace and join every worker before guard cleanup. Keep the 15-second correctness budget and exactly-one-row assertion; do not serialize the workers or raise the budget.

- [ ] **Step 5: Require repeatable serial and parallel passes**

Run:

```bash
RUST_TEST_THREADS=1 cargo test -p trail --lib
cargo test -p trail --lib
cargo test -p trail --lib
```

Expected: zero failures in all three runs; the two parallel runs prove cleanup does not depend on process restart.

- [ ] **Step 6: Commit test isolation**

```bash
git add trail/src/acp/capture.rs trail/src/db trail/src/lib.rs trail/src/test_support/scoped_state.rs
git commit -m "test: isolate process-global Trail fixtures"
```

---

### Task 9: Report native-COW space honestly

**Files:**
- Modify: `trail/src/model/reports/lane.rs`
- Modify: `trail/src/db/lane/workspace_view.rs`
- Modify: `trail/src/db/lane/workdir/lifecycle.rs`
- Modify: `trail/src/cli/command/render/lane/work.rs`
- Test: `trail/src/db/lane/workspace_view.rs`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Consumes: lane branch baseline root, materialized workdir, no-follow file walk, filesystem allocated blocks, materialization report/backend, and changed-path content identity.
- Produces: one `WorkspaceSpaceReport` for layered and native-COW lanes with logical, allocated, changed, clone, sharing-state, and evidence fields.

- [ ] **Step 1: Write RED native-COW space tests**

Spawn a native-COW lane, edit one file, add one file, delete one file, and assert `trail --json lane space` succeeds. On macOS require backend `native-cow`, exact logical/file counts, nonzero allocated bytes, changed bytes derived from content identity, clone count from materialization, and `physical_sharing` either `verified` with authenticated evidence or `unknown` with a nonempty reason.

- [ ] **Step 2: Run the focused test and verify RED**

Run:

```bash
cargo test -p trail --test e2e native_cow_lane_space_reports_allocated_and_changed_bytes_honestly -- --exact --nocapture
```

Expected: FAIL because `lane_workspace_space` currently rejects non-layered lanes.

- [ ] **Step 3: Extend the report contract**

Add:

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PhysicalSharing { Verified, NotShared, Unknown }

impl Default for PhysicalSharing {
    fn default() -> Self { Self::Unknown }
}

pub struct WorkspaceSpaceReport {
    pub view_id: String,
    pub logical_visible_bytes: u64,
    pub shared_physical_bytes: u64,
    pub lane_exclusive_physical_bytes: u64,
    pub shared_extent_bytes: Option<u64>,
    pub reclaimable_cache_bytes: u64,
    pub uncheckpointed_source_bytes: u64,
    pub generated_upper_bytes: u64,
    pub scratch_upper_bytes: u64,
    pub physical_accounting: String,
    pub backend: String,
    pub logical_file_count: u64,
    pub filesystem_allocated_bytes: u64,
    pub changed_since_baseline_bytes: Option<u64>,
    pub clone_count: u64,
    pub physical_sharing: PhysicalSharing,
    pub physical_sharing_evidence: String,
}
```

Use `#[serde(default)]` for the new fields where old serialized fixtures require compatibility. For native-COW lanes, `view_id` is the durable lane initialization ID; existing layered lanes retain their workspace view ID.

- [ ] **Step 4: Implement no-follow native-COW accounting**

Dispatch `lane_workspace_space` by workdir mode. Walk the materialized root without following symlinks, sum logical file length and Unix `blocks * 512` (or the platform allocation API), compare only changed-path content identities with the lane baseline to derive changed bytes, and read clone count/backend from durable materialization metadata. APFS block counts yield `PhysicalSharing::Unknown` and evidence `allocated_blocks_do_not_prove_apfs_extent_sharing` unless an authenticated extent API is added; never place them in `lane_exclusive_physical_bytes` as a proven value.

- [ ] **Step 5: Verify layered compatibility and native-COW JSON/human output**

Run:

```bash
cargo test -p trail db::lane::workspace_view::tests --lib -- --nocapture
cargo test -p trail --test e2e native_cow_lane_space -- --nocapture
```

Expected: PASS; layered fields retain their meanings and native-COW reports do not invent shared savings.

- [ ] **Step 6: Commit space observability**

```bash
git add trail/src/model/reports/lane.rs trail/src/db/lane trail/src/cli/command/render/lane/work.rs trail/tests/e2e.rs
git commit -m "feat: report native COW lane space"
```

---

### Task 10: Qualify and pin an available prolly revision reproducibly

**Files:**
- Modify: `prolly` (gitlink)
- Modify: `Cargo.lock`
- Modify: `docs/guides/performance-and-scale-benchmarks.md`

**Interfaces:**
- Consumes: reviewed available prolly commit `b68fcf4c57ce477292440da00bfc8aeed816c63a` and Trail storage/root-map/diff/merge/GC/recovery suites.
- Produces: an exact retained gitlink and lockfile that build from a clean recursive clone.

- [ ] **Step 1: Verify remote retention and inspect the exact dependency diff**

Run:

```bash
git -C prolly rev-parse HEAD
git -C prolly show --stat --oneline --decorate HEAD
git -C prolly fetch --dry-run origin b68fcf4c57ce477292440da00bfc8aeed816c63a
git diff --submodule=log -- prolly Cargo.lock
```

Expected: the exact `b68fcf4c57ce477292440da00bfc8aeed816c63a` object is fetchable from the configured remote and the dependency diff is understood; do not pin an abbreviated or unavailable object.

- [ ] **Step 2: Run focused compatibility before changing the recorded gitlink**

Run against the checked-out candidate:

```bash
cargo test -p trail db::storage --lib -- --nocapture
cargo test -p trail db::merge --lib -- --nocapture
cargo test -p trail db::core::fsck --lib -- --nocapture
cargo test -p trail db::change_ledger::recovery --lib -- --nocapture
```

Expected: PASS for SQLite and SlateDB root maps, diff/merge, GC/fsck, backup, and recovery paths.

- [ ] **Step 3: Regenerate the lockfile from the exact gitlink**

Run:

```bash
cargo update -p prolly
cargo metadata --locked --format-version 1 > /tmp/trail-cargo-metadata.json
```

Expected: `Cargo.lock` resolves the exact available prolly revision and `cargo metadata --locked` succeeds without changing unrelated dependency versions.

- [ ] **Step 4: Commit the qualified dependency pin**

```bash
git add prolly Cargo.lock docs/guides/performance-and-scale-benchmarks.md
git commit -m "build: pin retained prolly revision"
```

- [ ] **Step 5: Prove the committed recursive checkout builds**

Run:

```bash
trail_clone_root="$(mktemp -d /tmp/trail-clean-clone.XXXXXX)"
trail_candidate_commit="$(git rev-parse HEAD)"
git clone --no-local "$PWD" "$trail_clone_root/CrabDB"
git -C "$trail_clone_root/CrabDB" checkout "$trail_candidate_commit"
git -C "$trail_clone_root/CrabDB" submodule update --init --recursive
cargo build --manifest-path "$trail_clone_root/CrabDB/Cargo.toml" -p trail --locked
```

Expected: checkout fetches `b68fcf4c57ce477292440da00bfc8aeed816c63a` from the configured submodule remote and the locked build passes. Remove only the printed `/tmp/trail-clean-clone.XXXXXX` directory after validating it is the directory returned by `mktemp`. If checkout or build fails, fix the pin/lockfile in a new commit and repeat until the clean build passes; never amend away failed qualification evidence.

---

### Task 11: Add blocking scale and fault automation

**Files:**
- Create: `scripts/verify-real-repo-lane-scale.sh`
- Create: `scripts/test_verify_real_repo_lane_scale.py`
- Create: `scripts/check-real-repo-lane-scale.py`
- Create: `scripts/test_check_real_repo_lane_scale.py`
- Modify: `.github/workflows/ci.yml`
- Modify: `.github/workflows/scale.yml`
- Modify: `docs/guides/performance-and-scale-benchmarks.md`

**Interfaces:**
- Consumes: `TRAIL_BIN`, `TRAIL_SCALE_REPO`, `TRAIL_SCALE_LANES`, `TRAIL_SCALE_FILES_PER_LANE`, `TRAIL_SCALE_CONCURRENCY`, fault phase, and dedicated Git ref.
- Produces: per-command JSON, `results.tsv`, `metrics.json`, `faults.tsv`, exact expected-path manifests, stable threshold checking, and uploaded macOS/Linux evidence.

- [ ] **Step 1: Write RED harness and checker contract tests**

Test invalid options, lane-name uniqueness, disjoint path allocation, no native-COW fallback, concurrent failure propagation, committed-repair replay, exact metrics schema, missing evidence rejection, unexpected Git path rejection, and cleanup leakage. The Python tests use a fake Trail executable and a small temporary Git repository; they do not require Superset.

- [ ] **Step 2: Run contract tests and verify RED**

Run:

```bash
python3 -m unittest scripts/test_verify_real_repo_lane_scale.py -v
python3 -m unittest scripts/test_check_real_repo_lane_scale.py -v
```

Expected: FAIL because the new harness and checker do not exist.

- [ ] **Step 3: Implement the deterministic harness**

The shell harness must:

```text
1. Capture Trail/Git commits, filesystem, baseline Trail ref/root and Git HEAD/index.
2. Spawn N unique `native-cow` lanes concurrently and retain stdout/stderr/exit status.
3. Retry only identical fingerprints; classify committed repair separately from rollback.
4. Make at least FILES_PER_LANE disjoint edits, record concurrently, and verify isolation.
5. Collect lane space, status, readiness, handoff, initialization, lock, DB, observer-log,
   p50/p95/p99 wall time and peak RSS evidence.
6. Enqueue merges to one Trail `main`, run the queue, and verify the exact path manifest.
7. Export one mapped initial-to-final range to a dedicated `refs/heads/codex/...` Git ref.
8. Run conflict, dirty-Git refusal, cleanup, doctor, fsck, and Git fsck controls.
```

Use explicit validated paths, trap cleanup, and no `--force`, `--allow-stale`, `--allow-ignored`, or `--direct` bypass.

- [ ] **Step 4: Implement the complete fault matrix**

For each initialization phase, kill after the fsynced phase handshake and retry identically. Also cover daemon death, response loss after association/readiness, PID reuse, lock-holder crash, policy churn, filesystem replacement, disk full, permissions/fsync failure, conflicting lanes, and dirty Git export refusal. Each row records expected/actual code, durable phase, committed flag, retry result, integrity result, and leaked-resource count.

- [ ] **Step 5: Implement structural blocking checks**

`check-real-repo-lane-scale.py` rejects anything except: exact lane/edit counts; all initialization IDs unique by lane and stable across retries; zero ambiguous results/false deletions/missing lanes/unintended paths/integrity errors/live locks; `native-cow` for every lane; mapped-delta Git export; one Git commit; clean original branch/index; complete timing/RSS/DB/log/space metrics; and zero stale mount/socket/lock/initialization/materialization publications after cleanup. The 128-lane mode records latency without enforcing the 64-lane latency ceiling, but all correctness checks remain blocking.

- [ ] **Step 6: Wire platform automation**

Add serial and parallel `cargo test -p trail --lib` jobs to CI. Add macOS and Linux native changed-path integration jobs, schema/fault tests, a blocking 64-lane scheduled/release gate, and an artifact-producing 128-lane stress job. Upload raw JSON/TSV, exact manifests, environment metadata, and checker output on success or failure.

- [ ] **Step 7: Verify harness contracts locally**

Run:

```bash
python3 -m unittest scripts/test_verify_real_repo_lane_scale.py -v
python3 -m unittest scripts/test_check_real_repo_lane_scale.py -v
TRAIL_SCALE_LANES=4 TRAIL_SCALE_FILES_PER_LANE=5 TRAIL_SCALE_CONCURRENCY=4 scripts/verify-real-repo-lane-scale.sh
```

Expected: Python tests PASS and the smoke run produces a checker-approved artifact directory with no leaked resources.

- [ ] **Step 8: Commit release automation**

```bash
git add scripts/verify-real-repo-lane-scale.sh scripts/test_verify_real_repo_lane_scale.py scripts/check-real-repo-lane-scale.py scripts/test_check_real_repo_lane_scale.py .github/workflows/ci.yml .github/workflows/scale.yml docs/guides/performance-and-scale-benchmarks.md
git commit -m "test: gate concurrent lane production scale"
```

---

### Task 12: Execute final production qualification and publish an honest audit

**Files:**
- Modify: `docs/audits/2026-07-18-superset-trail-scale-verification.md`
- Create: `docs/audits/2026-07-18-trail-production-hardening.md`

**Interfaces:**
- Consumes: exact release binary, schema/fault evidence, serial/parallel tests, platform CI, 64-lane blocking artifact, 128-lane stress artifact, Trail doctor/fsck, Git fsck, and clean-clone build.
- Produces: an evidence-linked go/no-go audit with measured ceilings and every remaining limitation stated explicitly.

- [ ] **Step 1: Run formatting, lint, serial, parallel, and integration gates**

Run:

```bash
cargo fmt --all -- --check
cargo clippy -p trail --all-targets -- -D warnings
RUST_TEST_THREADS=1 cargo test -p trail --lib
cargo test -p trail --lib
cargo test -p trail --tests
cargo build -p trail --release --locked
```

Expected: zero formatting/lint/test/build failures. Save exact command, duration, host, and output artifacts.

- [ ] **Step 2: Run the complete schema and fault qualification**

Run:

```bash
cargo test -p trail --test schema_v19_lane_initializations -- --nocapture
cargo test -p trail --test lane_initialization_faults -- --nocapture
cargo test -p trail --test changed_path_ledger_macos -- --nocapture
```

On Linux run the corresponding native changed-path suite. Expected: all migration, downgrade, backup/restore, crash, corruption, producer inventory, lock, disk-full, conflict, and response-loss cases PASS.

- [ ] **Step 3: Run the 64-lane blocking Superset-class gate**

Run against `/Volumes/Workspace/Github/superset` with the exact release binary:

```bash
TRAIL_BIN="$PWD/target/release/trail" \
TRAIL_SCALE_REPO=/Volumes/Workspace/Github/superset \
TRAIL_SCALE_LANES=64 \
TRAIL_SCALE_FILES_PER_LANE=50 \
TRAIL_SCALE_CONCURRENCY=64 \
scripts/verify-real-repo-lane-scale.sh
```

Expected: checker PASS with 3,200 exact disjoint edits, zero ambiguous outcomes or leaked resources, one mapped Git commit on the dedicated ref, and unchanged user Git branch/index.

- [ ] **Step 4: Run the 128-lane stress gate**

Run the same command with `TRAIL_SCALE_LANES=128` and `TRAIL_SCALE_CONCURRENCY=128`.

Expected: all correctness/recoverability checks PASS; latency may exceed the blocking target but p50/p95/p99, RSS, DB/log growth, space, and retries are recorded.

- [ ] **Step 5: Verify post-workload integrity and clean build reproducibility**

Run `trail doctor`, `trail fsck`, `git fsck --full`, compare the expected Git tree and parent, verify no stale mount/socket/lock/initialization/materialization publication, and repeat the clean recursive checkout/build from Task 10 at the exact candidate commit.

- [ ] **Step 6: Write the go/no-go audit**

The new audit must list exact commits/binaries, platforms/filesystems, schema migration results, all test counts, each fault row, 64/128 timings and resources, logical/allocated/changed space with sharing evidence, Git integrity, fixed gaps, and every remaining issue. State “production-ready for the measured gate” only if every blocking check above is green; otherwise state “not production-ready” and name the blocking evidence without euphemism.

- [ ] **Step 7: Commit qualification documentation**

```bash
git add docs/audits/2026-07-18-superset-trail-scale-verification.md docs/audits/2026-07-18-trail-production-hardening.md
git commit -m "docs: publish Trail production qualification"
```
