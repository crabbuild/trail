# Activated Changed-Path Ledger Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make warm `status`, `diff`, and `record` proportional to authoritative changed-path candidates on large Linux and macOS repositories without ever returning false clean.

**Architecture:** A schema-v18-only `ChangedPathLedger` binds trust and source-sequenced evidence to a workspace/lane/view identity. A securely auto-started per-workspace daemon owns the qualified native observer, appends durable event segments, and reconciles automatically when continuity is unavailable; all readers use the same fenced snapshot API, and all filesystem producers use durable intents. Activation remains compiled off until the schema, recovery, producer inventory, Linux inotify suite, macOS FSEvents suite, and correctness/performance gates pass together.

**Tech Stack:** Rust 1.81, rusqlite/SQLite WAL, Prolly maps, clap, serde/CBOR, SHA-256, native inotify on Linux, native FSEvents on macOS, existing Trail daemon/REST/MCP surfaces, cargo-nextest-compatible tests.

## Global Constraints

- Schema version is exactly `18`; only explicit fresh `trail init` creates it.
- Existing version `0`, version `17`, versions greater than `18`, and partial or malformed version `18` return `SCHEMA_REINITIALIZE_REQUIRED` before any mutable store opens.
- A rejected open leaves `.trail` database files and sidecars byte-for-byte unchanged and instructs the user to back up and run `trail init --force`.
- There are no migrations, backfills, compatibility views, legacy trust states, or repair-on-open paths.
- Only `trusted` may prove clean; `reconciling`, `overflow`, `untrusted_gap`, `stale_baseline`, and `corrupt` fail closed.
- Linux and macOS must pass real native-adapter suites before activation; generic `notify` is advisory only.
- Unsupported platforms retain direct full observation and never persist a trusted ledger scope.
- Observer callbacks never acquire the workspace lock or primary SQLite connection.
- Observer segment and intent durability is explicit; SQLite remains WAL `synchronous=NORMAL`, so dead-owner restart reconciles unless a qualified durable cursor proves continuity.
- The first `status`, `diff`, or `record` securely discovers or starts exactly one daemon for the workspace.
- Stale, overflowed, corrupt, or unavailable ledger state automatically runs a full filesystem reconciliation and continues; `CHANGE_LEDGER_RECONCILE_REQUIRED` is returned only when startup, observation, or reconciliation cannot establish trust.
- Warm trusted paths must have zero full-scope walks, full root ranges, full SQLite index reads, legacy manifest I/O, upper recovery walks, external-adapter global work, and policy dependency rediscovery.
- Every task is test-first, independently reviewable, and committed separately; the final activation commit is the only commit that permits persisted trust to drive public reads.

---

## File Structure

### New changed-ledger module

- `trail/src/db/change_ledger/mod.rs`: public crate-level façade and the only entry point consumers call.
- `trail/src/db/change_ledger/types.rs`: identities, trust states, capabilities, cuts, evidence, snapshots, intents, and reconciliation types.
- `trail/src/db/change_ledger/store.rs`: SQLite persistence, binary path/prefix queries, caps, and CAS transitions.
- `trail/src/db/change_ledger/log.rs`: versioned segment codec, checksum/hash chain, rotation, durability offsets, and tail recovery.
- `trail/src/db/change_ledger/snapshot.rs`: live fence acquisition, tail folding, candidate snapshot, and sequence-aware acknowledgement.
- `trail/src/db/change_ledger/reconcile.rs`: streamed no-follow reconciliation and guarded publication.
- `trail/src/db/change_ledger/policy.rs`: authoritative recording policy fingerprint and persisted dependency manifest.
- `trail/src/db/change_ledger/intent.rs`: controlled-writer intent preparation and lifecycle transitions.
- `trail/src/db/change_ledger/recovery.rs`: owner, segment, intent, mirror, backup/restore, and GC recovery decisions.
- `trail/src/db/change_ledger/observer/mod.rs`: observer trait, capability qualification, owner lease, and platform selection.
- `trail/src/db/change_ledger/observer/linux.rs`: native recursive inotify adapter and sentinel fence.
- `trail/src/db/change_ledger/observer/macos.rs`: native FSEvents adapter, persistent event ID, root identity, and synchronous flush fence.

### Existing files with focused changes

- `trail/src/db/storage/schema/ddl.rs` and `trail/src/db/storage/schema/changed_path_ledger.rs`: exact one-shot v18 creation and validation.
- `trail/src/db/core/init.rs`: distinguish fresh creation from read-only preflight before Prolly/SQLite mutable opens.
- `trail/src/error.rs`: stable schema-reinitialize and reconcile-required errors.
- `trail/src/db/storage/worktree_index.rs`: remove the persisted daemon JSON cache as an authority; retain its full scan only as reconciliation/oracle code.
- `trail/src/cli/command/handler/daemon_rpc.rs` plus a new `daemon_start.rs`: authenticated per-workspace discovery/startup.
- `trail/src/db/core/status.rs`, `trail/src/db/record/diff.rs`, and `trail/src/db/record/recording/manual.rs`: consume one fenced ledger snapshot flow.
- `trail/src/db/record/checkout.rs`, `trail/src/db/lane/patching.rs`, `trail/src/db/lane/workdir/{materialize,sync,record}.rs`: controlled intents and lane scopes.
- `trail/src/db/lane/workdir/{view_journal,view_core}.rs` and `trail/src/db/lane/workspace_view.rs`: durable view intent journal and compact v2 marker.
- `trail/src/db/core/backup/{create,restore,verify}.rs` and `trail/src/db/storage/lifecycle/gc.rs`: fencing, epoch rotation, and intent GC roots.
- `trail/src/model/reports/{maintenance,worktree,lane}.rs`, CLI rendering, OpenAPI, server routes, and MCP tools: structured diagnostics and metrics.
- `trail/tests/changed_path_ledger_*.rs`, `trail/tests/e2e.rs`, and scale scripts: hard-cutover, state/property, real-adapter, crash, E2E, and locality gates.

---

### Task 1: Replace migration prototype with exact v18 hard cutover

**Files:**
- Modify: `trail/src/db/mod.rs`
- Modify: `trail/src/db/core/init.rs`
- Modify: `trail/src/db/storage/schema.rs`
- Modify: `trail/src/db/storage/schema/ddl.rs`
- Replace: `trail/src/db/storage/schema/changed_path_ledger.rs`
- Modify: `trail/src/error.rs`
- Test: `trail/tests/e2e.rs`
- Create: `trail/tests/schema_v18_hard_cutover.rs`

**Interfaces:**
- Consumes: existing `Trail::init_with_options`, `Trail::open_at_without_recovery`, and `TRAIL_SCHEMA_VERSION`.
- Produces: `SchemaOpenMode::{FreshCreate, Existing}`, `preflight_existing_schema(&Path) -> Result<()>`, `create_schema_v18(&Connection) -> Result<()>`, `validate_schema_v18(&Connection) -> Result<()>`, and `Error::SchemaReinitializeRequired { found: String, guidance: String }`.

- [ ] **Step 1: Write rejection and no-mutation tests before changing open behavior**

```rust
#[test]
fn existing_v17_is_rejected_without_mutating_any_trail_byte() {
    let fixture = SchemaFixture::versioned(17);
    let before = fixture.snapshot_tree_bytes();
    let err = Trail::open(fixture.root()).unwrap_err();
    assert_eq!(err.code(), "SCHEMA_REINITIALIZE_REQUIRED");
    assert!(err.to_string().contains("trail init --force"));
    assert_eq!(fixture.snapshot_tree_bytes(), before);
}

#[test]
fn partial_v18_is_rejected_without_repair() {
    let fixture = SchemaFixture::partial_v18("changed_path_scopes");
    let before = fixture.snapshot_tree_bytes();
    let err = Trail::open(fixture.root()).unwrap_err();
    assert_eq!(err.code(), "SCHEMA_REINITIALIZE_REQUIRED");
    assert_eq!(fixture.snapshot_tree_bytes(), before);
}
```

- [ ] **Step 2: Run the focused hard-cutover tests and capture the expected failure**

Run: `cargo test -p trail --test schema_v18_hard_cutover -- --nocapture`

Expected: FAIL because existing schemas are migrated/repaired or mutable sidecars are opened before validation.

- [ ] **Step 3: Add the stable error contract**

```rust
#[error("workspace schema {found} cannot be opened; {guidance}")]
SchemaReinitializeRequired { found: String, guidance: String },
#[error("changed-path ledger reconciliation required for {scope}: {reason}; run `{command}`")]
ChangeLedgerReconcileRequired {
    scope: String,
    state: String,
    reason: String,
    command: String,
},
```

Map these to `SCHEMA_REINITIALIZE_REQUIRED`/exit `15` and `CHANGE_LEDGER_RECONCILE_REQUIRED`/exit `16`; add unit assertions for both codes, exit statuses, and exact recovery commands.

- [ ] **Step 4: Introduce explicit open modes and a read-only preflight**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SchemaOpenMode { FreshCreate, Existing }

fn preflight_existing_schema(db_path: &Path) -> Result<()> {
    let flags = rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY
        | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = rusqlite::Connection::open_with_flags(db_path, flags)
        .map_err(schema_reinitialize_error)?;
    validate_schema_v18(&conn).map_err(schema_reinitialize_error)
}

fn schema_reinitialize_error(err: impl std::fmt::Display) -> Error {
    Error::SchemaReinitializeRequired {
        found: err.to_string(),
        guidance: "back up this workspace, then run `trail init --force` to create schema v18".into(),
    }
}
```

Call `preflight_existing_schema` before `open_prolly_store`, before opening a read-write SQLite connection, and before creating any directory or sidecar. Only `Trail::init_with_options` passes `FreshCreate`; remove its redundant second `init_schema` call.

- [ ] **Step 5: Replace migration/backfill/repair DDL with a one-savepoint fresh creator**

```rust
pub(crate) fn create_schema_v18(conn: &Connection) -> Result<()> {
    if conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))? != 0 {
        return Err(Error::Corrupt("fresh schema connection is not empty".into()));
    }
    conn.execute_batch("SAVEPOINT create_v18;")?;
    let result = (|| {
        conn.execute_batch(BASE_SCHEMA_V18)?;
        conn.execute_batch(CHANGED_PATH_LEDGER_SCHEMA_V18)?;
        validate_schema_v18_shape(conn)?;
        conn.execute("INSERT INTO schema_meta(key,value) VALUES('schema_version','18')", [])?;
        conn.pragma_update(None, "user_version", 18_i64)?;
        validate_schema_v18(conn)
    })();
    match result {
        Ok(()) => conn.execute_batch("RELEASE create_v18;").map_err(Into::into),
        Err(err) => { let _ = conn.execute_batch("ROLLBACK TO create_v18; RELEASE create_v18;"); Err(err) }
    }
}
```

The final ledger DDL must include scopes, entries, prefixes, policy dependencies, intents, intent paths, intent prefixes, reconciliations, observer segments, and observer owners with exact checks/FKs/unique indexes and binary path collation. Do not use `IF NOT EXISTS` in creation or validation.

- [ ] **Step 6: Add exact-shape validation tests**

Validate `sqlite_master.sql`, `PRAGMA table_xinfo`, `foreign_key_list`, `index_xinfo`, check/default clauses, schema metadata, `user_version=18`, and the observer log format min/max. Mutate one attribute per test and assert read-only rejection.

- [ ] **Step 7: Run schema and open-path tests**

Run: `cargo test -p trail --test schema_v18_hard_cutover && cargo test -p trail db::storage::schema error::tests`

Expected: PASS; snapshots for v0/v17/v19/partial-v18 are byte-identical before and after attempted open.

- [ ] **Step 8: Commit only the hard-cutover slice**

```bash
git add trail/src/db/mod.rs trail/src/db/core/init.rs trail/src/db/storage/schema.rs trail/src/db/storage/schema/ddl.rs trail/src/db/storage/schema/changed_path_ledger.rs trail/src/error.rs trail/tests/e2e.rs trail/tests/schema_v18_hard_cutover.rs
git commit -m "feat: enforce schema v18 hard cutover"
```

### Task 2: Add typed ledger state and exact SQLite CAS persistence

**Files:**
- Create: `trail/src/db/change_ledger/mod.rs`
- Create: `trail/src/db/change_ledger/types.rs`
- Create: `trail/src/db/change_ledger/store.rs`
- Modify: `trail/src/db/mod.rs`
- Create: `trail/tests/changed_path_ledger_state.rs`

**Interfaces:**
- Consumes: v18 tables from Task 1, existing `ObjectId`, `ChangeId`, and ref generation/root values.
- Produces: `ChangedPathLedger<'a>`, `ScopeId`, `ScopeIdentity`, `BaselineIdentity`, `ProviderCapabilities`, `TrustState`, `EvidenceCut`, `CandidateSnapshot`, `ExpectedScope`, `begin_scope`, `mark_prefix_dirty`, and CAS methods used by every later task.

- [ ] **Step 1: Write state-machine and stale-CAS tests**

```rust
#[test]
fn only_trusted_scope_can_return_an_authoritative_snapshot() {
    for state in [TrustState::Reconciling, TrustState::Overflow,
        TrustState::UntrustedGap, TrustState::StaleBaseline, TrustState::Corrupt] {
        let store = fixture_with_state(state);
        assert!(matches!(store.snapshot_candidates(expected()), Err(Error::ChangeLedgerReconcileRequired { .. })));
    }
}

#[test]
fn stale_epoch_or_ref_cannot_advance_baseline() {
    let store = trusted_fixture();
    assert_eq!(store.advance_baseline(stale_expected(), target()).unwrap_err().code(),
        "CHANGE_LEDGER_RECONCILE_REQUIRED");
}
```

- [ ] **Step 2: Run the state tests to verify missing APIs fail**

Run: `cargo test -p trail --test changed_path_ledger_state`

Expected: compile failure for missing `change_ledger` types and methods.

- [ ] **Step 3: Define serializable identities, capabilities, and states**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum TrustState { Trusted, Reconciling, Overflow, UntrustedGap, StaleBaseline, Corrupt }

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ProviderCapabilities {
    pub durable_cursor: bool,
    pub linearizable_fence: bool,
    pub rename_pairing: bool,
    pub overflow_scope: bool,
    pub filesystem_supported: bool,
    pub clean_proof_allowed: bool,
    pub power_loss_durability: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, Serialize, Deserialize)]
pub(crate) struct ScopeId(pub [u8; 32]);
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) enum ScopeKind { Workspace, MaterializedLane, WorkspaceView }
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Serialize, Deserialize)]
pub(crate) struct LedgerPath(pub String);
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ScopeIdentity { pub scope_id: ScopeId, pub kind: ScopeKind, pub owner_id: String }
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BaselineIdentity { pub ref_name: String, pub ref_generation: u64, pub change_id: ChangeId, pub root_id: ObjectId }
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PolicyIdentity { pub fingerprint: [u8; 32], pub generation: u64 }
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct FilesystemIdentity(pub Vec<u8>);
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ProviderIdentity { pub identity: Vec<u8>, pub capabilities: ProviderCapabilities }
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct EvidenceCut { pub source: EvidenceSource, pub sequence: u64, pub durable_offset: u64 }
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct DirtyPrefix { pub path: LedgerPath, pub complete: bool, pub reason: String, pub first_sequence: u64, pub last_sequence: u64 }
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct OwnedEvidence { pub source: EvidenceSource, pub through_sequence: u64, pub exact_paths: Vec<LedgerPath>, pub prefixes: Vec<DirtyPrefix> }
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CandidateSnapshot { pub expected: ExpectedScope, pub cut: EvidenceCut, pub exact_paths: Vec<LedgerPath>, pub prefixes: Vec<DirtyPrefix>, pub trust: TrustState }

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ExpectedScope {
    pub scope_id: ScopeId,
    pub epoch: u64,
    pub ref_name: String,
    pub ref_generation: u64,
    pub baseline_root: ObjectId,
    pub policy_fingerprint: [u8; 32],
    pub policy_generation: u64,
    pub filesystem_identity: Vec<u8>,
    pub provider_identity: Vec<u8>,
}
```

Also define `EvidenceSource::{Observer, Intent, Reconciliation, GitAdvisory}` and a bitmask `EvidenceFlags` for create/content/mode/delete/rename-from/rename-to. `LedgerPath::parse` rejects absolute, parent-escaping, NUL, and unsupported/non-UTF-8 inputs and preserves exact case.

- [ ] **Step 4: Implement one façade and CAS store operations**

```rust
pub(crate) struct ChangedPathLedger<'a> { conn: &'a Connection }

impl<'a> ChangedPathLedger<'a> {
    pub(crate) fn new(conn: &'a Connection) -> Self { Self { conn } }
    pub(crate) fn begin_scope(&self, identity: &ScopeIdentity, baseline: &BaselineIdentity, policy: &PolicyIdentity, filesystem: &FilesystemIdentity, provider: &ProviderIdentity) -> Result<ScopeId>;
    pub(crate) fn mark_prefix_dirty(&self, expected: &ExpectedScope, prefix: &DirtyPrefix) -> Result<()>;
    pub(crate) fn mark_untrusted(&self, expected: &ExpectedScope, state: TrustState, reason: &str) -> Result<()>;
    pub(crate) fn snapshot_candidates(&self, expected: &ExpectedScope) -> Result<CandidateSnapshot>;
    pub(crate) fn acknowledge(&self, expected: &ExpectedScope, cut: &EvidenceCut, owned: &OwnedEvidence) -> Result<()>;
    pub(crate) fn advance_baseline(&self, expected: &ExpectedScope, target: &BaselineIdentity, cut: &EvidenceCut) -> Result<()>;
}
```

`begin_scope` inserts `untrusted_gap` with a fresh epoch and cannot accept `trusted` from its caller. Only reconciliation publication may perform the initial promotion.

Every update includes `WHERE scope_id=? AND epoch=? AND ref_generation=? AND baseline_root=? AND policy_fingerprint=? AND filesystem_identity=? AND provider_identity=?`; zero changed rows is a stale CAS, never success.

- [ ] **Step 5: Implement binary exact/prefix evidence and caps**

Use half-open binary ranges `(prefix, prefix_successor)` and indexed `COLLATE BINARY`, merge overlapping complete prefixes while retaining the earliest/latest source sequence and completeness reason, and atomically mark `overflow` when persisted entry/prefix/byte/tail caps are exceeded.

- [ ] **Step 6: Add property tests for event order, coalescing, and acknowledgement**

```rust
proptest! {
    #[test]
    fn acknowledgement_never_clears_later_or_other_source_evidence(events in event_sequences()) {
        let (store, c1) = apply_events_and_cut(events);
        store.acknowledge(&expected(), &c1, &owned_before(c1)).unwrap();
        prop_assert!(store.all_remaining().iter().all(|e| e.sequence > c1.sequence || e.source != c1.source));
    }
}
```

- [ ] **Step 7: Run and commit the typed store**

Run: `cargo test -p trail --test changed_path_ledger_state`

Expected: PASS for every trust transition, stale CAS, prefix coalescing, and sequence boundary.

```bash
git add trail/src/db/mod.rs trail/src/db/change_ledger trail/tests/changed_path_ledger_state.rs
git commit -m "feat: add changed-path ledger state store"
```

### Task 3: Implement the durable observer segment protocol

**Files:**
- Create: `trail/src/db/change_ledger/log.rs`
- Create: `trail/tests/changed_path_ledger_log.rs`
- Modify: `trail/Cargo.toml`

**Interfaces:**
- Consumes: `ScopeId`, `LedgerPath`, `EvidenceFlags`, and `EvidenceSource` from Task 2.
- Produces: `SegmentWriter::acquire`, `SegmentWriter::append`, `SegmentWriter::flush_durable`, `SegmentWriter::rotate`, `recover_segments`, `DurableCut`, and `RecoveredTail`.

- [ ] **Step 1: Write codec, torn-tail, hash-chain, and retired-owner tests**

```rust
#[test]
fn torn_tail_recovers_only_through_last_checked_record() {
    let mut bytes = encoded_segment(&[event(1), event(2)]);
    bytes.truncate(bytes.len() - 7);
    let recovered = recover_bytes(&bytes).unwrap();
    assert_eq!(recovered.records, vec![event(1)]);
    assert!(recovered.requires_reconciliation);
}

#[test]
fn retired_owner_cannot_append_to_reused_epoch() {
    let first = fixture.acquire(epoch(3), owner("a")).unwrap();
    fixture.replace_owner(epoch(4), owner("b")).unwrap();
    assert!(first.append(&event(1)).is_err());
}
```

- [ ] **Step 2: Run the log tests and verify they fail before implementation**

Run: `cargo test -p trail --test changed_path_ledger_log`

Expected: compile failure for missing segment protocol.

- [ ] **Step 3: Define a bounded, versioned on-disk record format**

```rust
const SEGMENT_MAGIC: &[u8; 8] = b"TRAILCPL";
const LOG_FORMAT_VERSION: u16 = 1;
const MAX_RECORD_BYTES: u32 = 1024 * 1024;

#[derive(Serialize, Deserialize)]
struct SegmentHeader {
    format: u16,
    scope_id: ScopeId,
    epoch: u64,
    owner_token: [u8; 32],
    provider_cursor: Vec<u8>,
    previous_segment_hash: [u8; 32],
}
```

Encode `length | sequence | source | payload | previous_record_hash | checksum`; reject unsupported format, non-monotonic sequence, over-limit length, checksum mismatch, broken link, owner mismatch, and segment lineage mismatch.

- [ ] **Step 4: Implement exclusive owner lease, append, group flush, and rotation**

The writer verifies the lease token before each batch, syncs record bytes before publishing `durable_end_offset`, syncs the old segment and directory before publishing the next header, and revokes trust in memory immediately on append/flush/disk-full/lease/heartbeat failure.

- [ ] **Step 5: Implement conservative tail recovery**

```rust
pub(crate) struct RecoveredTail {
    pub records: Vec<ObserverRecord>,
    pub durable_end: u64,
    pub last_sequence: u64,
    pub last_hash: [u8; 32],
    pub requires_reconciliation: bool,
}
```

Never skip a corrupt middle record. A partial final record is discarded and marks the scope untrusted. Recovery must bound records and bytes before allocation.

- [ ] **Step 6: Run fault tests and commit**

Run: `cargo test -p trail --test changed_path_ledger_log`

Expected: PASS for clean recovery, torn writes, corrupt checksum/linkage, disk-full injection, rotation crash points, stale owner, and cap exhaustion.

```bash
git add trail/Cargo.toml trail/src/db/change_ledger/log.rs trail/tests/changed_path_ledger_log.rs
git commit -m "feat: add durable changed-path observer log"
```

### Task 4: Persist policy dependencies before filtering

**Files:**
- Create: `trail/src/db/change_ledger/policy.rs`
- Modify: `trail/src/db/storage/worktree_index.rs`
- Create: `trail/tests/changed_path_ledger_policy.rs`

**Interfaces:**
- Consumes: v18 `changed_path_policy_dependencies`, `ExpectedScope`, existing `RecordingPolicySnapshot`, and Trail/Git ignore walkers.
- Produces: `CompiledPolicy`, `PolicyDependency`, `PolicyManifest`, `compile_policy`, `validate_policy_manifest`, and `raw_event_invalidates_policy`.

- [ ] **Step 1: Write nested/global/config dependency invalidation tests**

```rust
#[test]
fn raw_nested_ignore_event_stales_scope_before_ignore_filtering() {
    let policy = fixture.compile_policy().unwrap();
    assert!(raw_event_invalidates_policy(&policy, path("src/.gitignore")));
}

#[test]
fn unchanged_manifest_reuses_policy_without_tree_discovery() {
    let (policy, metrics) = fixture.load_policy_twice().unwrap();
    assert_eq!(policy.fingerprint, fixture.expected_fingerprint());
    assert_eq!(metrics.policy_dependency_full_discovery, 1);
}
```

- [ ] **Step 2: Run the focused policy tests**

Run: `cargo test -p trail --test changed_path_ledger_policy`

Expected: FAIL because dependencies are not persisted or raw events are filtered first.

- [ ] **Step 3: Define the authoritative manifest**

```rust
pub(crate) struct PolicyDependency {
    pub identity: String,
    pub kind: PolicyDependencyKind,
    pub content_identity: [u8; 32],
    pub metadata_identity: Vec<u8>,
    pub observable: bool,
    pub generation: u64,
    pub last_source_sequence: u64,
}

pub(crate) struct CompiledPolicy {
    pub snapshot: RecordingPolicySnapshot,
    pub fingerprint: [u8; 32],
    pub dependencies: Vec<PolicyDependency>,
    pub adapter_equivalence: AdapterEquivalence,
}
```

Cover built-in rule version, Trail configuration, nested `.trailignore`/`.gitignore`, `.git/info/exclude`, `core.excludesFile`, included Git config, global/system config, normalization, mode/symlink handling, and case sensitivity.

- [ ] **Step 4: Persist and validate manifest identities**

External or unobservable dependencies set `stale_baseline`; observers route raw dependency events to invalidation before normal path filtering. Reuse is allowed only when every stored identity still validates through bounded direct checks.

- [ ] **Step 5: Run policy tests and commit**

Run: `cargo test -p trail --test changed_path_ledger_policy`

Expected: PASS including nested changes, global config changes, included config, normalization/mode/case changes, and zero repeat discovery.

```bash
git add trail/src/db/change_ledger/policy.rs trail/src/db/storage/worktree_index.rs trail/tests/changed_path_ledger_policy.rs
git commit -m "feat: persist changed-path policy dependencies"
```

### Task 5: Stream full reconciliation and publish by exact CAS

**Files:**
- Create: `trail/src/db/change_ledger/reconcile.rs`
- Modify: `trail/src/db/storage/worktree_index.rs`
- Modify: `trail/src/db/storage/root_diff.rs`
- Modify: `trail/src/model/reports/maintenance.rs`
- Create: `trail/tests/changed_path_ledger_reconcile.rs`

**Interfaces:**
- Consumes: `ChangedPathLedger`, `ExpectedScope`, `CompiledPolicy`, provider `ObserverFence`, and existing worktree/root readers.
- Produces: `begin_reconciliation(...) -> ReconciliationAttempt`, `reconcile_full(...)`, `ReconciliationAttempt::observe`, `ReconciliationAttempt::publish`, `ReconcileMode::{Full, ProvenPrefixes}`, and `ChangeLedgerReconcileReport`.

- [ ] **Step 1: Write correctness and race tests against a full-scan oracle**

```rust
#[test]
fn reconciliation_finds_add_modify_mode_delete_and_rename() {
    let expected = fixture.mutate_all_path_kinds();
    let report = fixture.reconcile_full().unwrap();
    assert_eq!(fixture.ledger_candidates(), fixture.full_scan_oracle());
    assert_eq!(report.observed_candidates, expected);
}

#[test]
fn ref_or_filesystem_replacement_during_scan_cannot_publish_trust() {
    let attempt = fixture.begin_reconciliation().unwrap();
    fixture.replace_scope_root();
    assert!(attempt.publish().is_err());
    assert_ne!(fixture.trust_state(), TrustState::Trusted);
}
```

- [ ] **Step 2: Run the reconciliation tests to establish red**

Run: `cargo test -p trail --test changed_path_ledger_reconcile`

Expected: compile failure for missing `ReconciliationAttempt` and `reconcile_full`.

- [ ] **Step 3: Start observation before enumeration and stream staging rows**

```rust
pub(crate) struct ReconcileExpectation {
    pub scope: ExpectedScope,
    pub start_fence: ObserverFence,
    pub root_handle_identity: Vec<u8>,
}

pub(crate) fn reconcile_full(
    ledger: &ChangedPathLedger<'_>,
    observer: &dyn QualifiedObserver,
    expected: &ReconcileExpectation,
    policy: &CompiledPolicy,
) -> Result<ChangeLedgerReconcileReport>;
```

Open the root no-follow, record identity, enumerate through bounded buffers, hash relevant regular files, verify each before/after identity, stream rows into the attempt staging table, and find deletions by streaming the baseline Prolly range. Never retain the complete repository manifest in memory.

- [ ] **Step 4: Fence, drain, and publish only on the full identity tuple**

Obtain the end fence after enumeration, fold all evidence through it into staging, revalidate ref/root/epoch/policy generation/filesystem/provider/root-handle identity, then replace authoritative candidates and transition to `trusted` in one transaction. A mismatch abandons staging and retries; it cannot partially publish.

- [ ] **Step 5: Limit prefix reconciliation to provider-proven containment**

`ProvenPrefixes` accepts only persisted complete prefix evidence from a qualified provider. Overflow, owner loss, global policy change, corrupt log, and unknown gaps force `Full`; user-provided prefixes may refresh rows but never promote global trust.

- [ ] **Step 6: Run reconciliation, memory, and race tests**

Run: `cargo test -p trail --test changed_path_ledger_reconcile`

Expected: PASS with complete oracle equality, no false clean under injected races, and bounded peak staging memory for 100,000 fixture paths.

- [ ] **Step 7: Commit the reconciler**

```bash
git add trail/src/db/change_ledger/reconcile.rs trail/src/db/storage/worktree_index.rs trail/src/db/storage/root_diff.rs trail/src/model/reports/maintenance.rs trail/tests/changed_path_ledger_reconcile.rs
git commit -m "feat: add streamed changed-path reconciliation"
```

### Task 6: Add durable controlled-writer intents, recovery, backup, and GC roots

**Files:**
- Create: `trail/src/db/change_ledger/intent.rs`
- Create: `trail/src/db/change_ledger/recovery.rs`
- Modify: `trail/src/db/storage/lifecycle/gc.rs`
- Modify: `trail/src/db/core/backup/create.rs`
- Modify: `trail/src/db/core/backup/verify.rs`
- Modify: `trail/src/db/core/backup/restore.rs`
- Create: `trail/tests/changed_path_ledger_recovery.rs`

**Interfaces:**
- Consumes: `ExpectedScope`, ledger store CAS, observer fence, operation/root IDs, authoritative ref transaction, and backup/GC infrastructure.
- Produces: `IntentId`, `IntentProducer`, `IntentTarget`, `IntentEvidence`, `VerifiedFilesystemCut`, `IntentState`, `prepare_intent`, `mark_filesystem_applied`, `publish_intent`, `recover_scope`, `ChangedPathLedger::recover`, and `ledger_gc_roots`.

- [ ] **Step 1: Write lifecycle, crash-boundary, restore, and GC tests**

```rust
#[test]
fn concurrent_same_path_event_survives_intent_acknowledgement() {
    let intent = fixture.prepare_checkout_intent("src/lib.rs").unwrap();
    fixture.mark_filesystem_applied(&intent).unwrap();
    fixture.append_external_event_after_intent("src/lib.rs").unwrap();
    fixture.publish_and_ack(&intent).unwrap();
    assert!(fixture.candidates().contains("src/lib.rs"));
}

#[test]
fn prepared_target_is_a_gc_root_until_terminal_recovery() {
    let intent = fixture.prepare_new_target().unwrap();
    fixture.gc().unwrap();
    assert!(fixture.target_exists(&intent));
    fixture.abort_and_recover(intent).unwrap();
    fixture.gc().unwrap();
    assert!(!fixture.target_exists(&intent));
}
```

- [ ] **Step 2: Run the recovery tests and verify missing behavior**

Run: `cargo test -p trail --test changed_path_ledger_recovery`

Expected: FAIL because mutations have no durable intent lifecycle or GC roots.

- [ ] **Step 3: Define producer and lifecycle types**

```rust
pub(crate) enum IntentProducer {
    Checkout, LaneSync, Materialize, StructuredPatchProjection,
    RestoreProjection, CowPublication, ObservedCheckpoint,
}
pub(crate) enum IntentState { Prepared, FilesystemApplied, Published, Acknowledged, Aborted }
pub(crate) struct IntentTarget {
    pub change_id: ChangeId,
    pub root_id: ObjectId,
    pub operation_id: Option<ObjectId>,
}
pub(crate) struct IntentEvidence {
    pub exact_paths: Vec<LedgerPath>,
    pub complete_prefixes: Vec<DirtyPrefix>,
}
pub(crate) struct VerifiedFilesystemCut {
    pub observer_cut: EvidenceCut,
    pub verified_paths: u64,
    pub filesystem_identity: Vec<u8>,
}
```

Persist expected epoch/ref generation/root, target, start cursor, exact paths/prefixes, and source ownership. Sync the intent file before publishing each durable lifecycle transition.

- [ ] **Step 4: Implement recovery decisions before mutation and GC**

```rust
pub(crate) enum RecoveryDecision {
    FinishPublication,
    RetainCandidatesAndAcknowledge,
    Abort,
    FullReconciliation,
}
```

Compare intent state, verified filesystem, authoritative ref/operation, provider cut, and ledger baseline. Ambiguity retains candidates or reconciles; it never clears evidence. Run `recover_scope` before another mutation, snapshot, backup, or GC.

- [ ] **Step 5: Fence backup and rotate restored epochs**

Backup either stores a matching fenced SQL+segment cut or marks every backed-up scope untrusted. Restore rotates epoch, clears owner/cursor, discards unmatched tails, rebinds filesystem identity, and writes `untrusted_gap`; it never restores `trusted`.

- [ ] **Step 6: Add intent targets to GC and retire scopes safely**

Prepared and published target roots/operations remain GC roots until a terminal state. Lane/view deletion first retires the scope and revokes owner lease transactionally, then removes segments after no reader can reference them.

- [ ] **Step 7: Execute the crash matrix and commit**

Run: `cargo test -p trail --test changed_path_ledger_recovery`

Expected: PASS for process kill before/after intent sync, filesystem sync, operation write, ref CAS, ledger CAS, mirror repair, backup rotation, restore to another filesystem, and GC on both sides of recovery.

```bash
git add trail/src/db/change_ledger/intent.rs trail/src/db/change_ledger/recovery.rs trail/src/db/storage/lifecycle/gc.rs trail/src/db/core/backup trail/tests/changed_path_ledger_recovery.rs
git commit -m "feat: recover changed-path intents and lifecycle"
```

### Task 7: Qualify the native Linux inotify observer

**Files:**
- Create: `trail/src/db/change_ledger/observer/mod.rs`
- Create: `trail/src/db/change_ledger/observer/linux.rs`
- Modify: `trail/Cargo.toml`
- Modify: root `Cargo.toml`
- Create: `trail/tests/changed_path_ledger_linux.rs`

**Interfaces:**
- Consumes: `SegmentWriter`, `ObserverRecord`, `ScopeIdentity`, root handle identity, and `ProviderCapabilities`.
- Produces: `QualifiedObserver` trait, `ObserverFence`, `ObserverLease`, `LinuxInotifyObserver`, and `select_observer`.

- [ ] **Step 1: Write Linux-only real filesystem qualification tests**

```rust
#[cfg(target_os = "linux")]
#[test]
fn recursive_directory_creation_is_covered_before_children_can_be_clean() {
    let observer = real_observer_fixture();
    fixture.create_dir_and_child_while_watch_is_added("a/b/file");
    let cut = observer.fence().unwrap();
    assert!(observer.fold_through(&cut).unwrap().covers("a/b/file"));
}

#[cfg(target_os = "linux")]
#[test]
fn queue_overflow_revokes_clean_proof() {
    let observer = overflow_fixture();
    assert_eq!(observer.fence().unwrap_err().reason(), "inotify_queue_overflow");
}
```

- [ ] **Step 2: Run on Linux and confirm the adapter is absent**

Run: `cargo test -p trail --test changed_path_ledger_linux -- --nocapture`

Expected: compile failure for missing `LinuxInotifyObserver`.

- [ ] **Step 3: Add a direct `inotify` dependency and observer contract**

```rust
pub(crate) trait QualifiedObserver: Send + Sync {
    fn capabilities(&self) -> ProviderCapabilities;
    fn root_identity(&self) -> Result<Vec<u8>>;
    fn fence(&self) -> Result<ObserverFence>;
    fn flush_durable(&self, fence: &ObserverFence) -> Result<DurableCut>;
    fn shutdown(self: Box<Self>) -> Result<()>;
}
```

The generic `notify` adapter implements an advisory type but always returns `clean_proof_allowed=false`.

- [ ] **Step 4: Implement recursive coverage, rename cookies, and race handling**

Add watches root-first; for directory creation/move-in, install the watch then enumerate the new subtree and mark its parent as a complete dirty prefix. Pair `MOVED_FROM`/`MOVED_TO` cookies while retaining both endpoints; expiration becomes a conservative complete parent prefix. `IN_Q_OVERFLOW`, `IN_IGNORED`, unknown watch descriptor, root deletion, and watch-add failure revoke trust globally.

- [ ] **Step 5: Implement a sentinel delivery fence**

Create and sync a nonce-named sentinel inside the pinned scope, wait until its exact inotify sequence is durably appended, unlink it, wait for the unlink record, then return the covered cut. The observer qualifies only after a reconciliation began with watching active and ended through this fence.

- [ ] **Step 6: Run the real Linux suite**

Run: `cargo test -p trail --test changed_path_ledger_linux -- --nocapture`

Expected: PASS for create/delete/content/mode, file/directory/case rename, recursive add races, delayed backlog, rename storms, overflow, owner death, filesystem/root replacement, and fence ordering on a native Linux filesystem.

- [ ] **Step 7: Commit the Linux adapter without enabling authority**

```bash
git add Cargo.toml trail/Cargo.toml trail/src/db/change_ledger/observer trail/tests/changed_path_ledger_linux.rs
git commit -m "feat: add qualified Linux change observer"
```

### Task 8: Qualify the native macOS FSEvents observer

**Files:**
- Create: `trail/src/db/change_ledger/observer/macos.rs`
- Modify: `trail/src/db/change_ledger/observer/mod.rs`
- Modify: `trail/Cargo.toml`
- Modify: root `Cargo.toml`
- Create: `trail/tests/changed_path_ledger_macos.rs`

**Interfaces:**
- Consumes: `QualifiedObserver`, `SegmentWriter`, scope/root identity, provider cursors, and reconciliation activation contract.
- Produces: `MacOsFseventsObserver` with persisted event-ID continuity and synchronous flush fence.

- [ ] **Step 1: Write macOS-only real FSEvents tests**

```rust
#[cfg(target_os = "macos")]
#[test]
fn must_scan_subdirs_revokes_cursor_resume() {
    let observer = fsevents_fixture_with_flag(kFSEventStreamEventFlagMustScanSubDirs);
    assert!(observer.resume().is_err());
    assert_eq!(fixture.trust_state(), TrustState::UntrustedGap);
}

#[cfg(target_os = "macos")]
#[test]
fn synchronous_flush_returns_a_durably_foldable_event_id() {
    let observer = real_observer_fixture();
    fixture.write("src/lib.rs", b"new");
    let cut = observer.fence().unwrap();
    assert!(cut.provider_cursor.event_id >= fixture.write_event_id());
}
```

- [ ] **Step 2: Run on macOS and verify the adapter is absent**

Run: `cargo test -p trail --test changed_path_ledger_macos -- --nocapture`

Expected: compile failure for missing `MacOsFseventsObserver`.

- [ ] **Step 3: Add a direct FSEvents binding and file-events stream**

Request file-level events, persist `FSEventStreamEventId`, device/root identity, stream creation identity, and capability record. Normalize all event paths relative to the pinned root before appending; non-UTF-8 or escaped-root paths revoke qualification.

- [ ] **Step 4: Enforce continuity and gap flags**

`MustScanSubDirs`, `UserDropped`, `KernelDropped`, `EventIdsWrapped`, `RootChanged`, `Unmount`, `HistoryDone` inconsistency, root identity change, or a resume ID older than available history marks `untrusted_gap` and starts full reconciliation. Resume without reconciliation is allowed only when event-ID and root continuity are proven.

- [ ] **Step 5: Implement synchronous flush fencing**

Call the synchronous stream flush, wait until every callback through the returned/latest event ID has been durably appended, then return that cursor. Persisted cursor alone never qualifies power-loss durability.

- [ ] **Step 6: Run the real macOS suite**

Run: `cargo test -p trail --test changed_path_ledger_macos -- --nocapture`

Expected: PASS for create/delete/content/mode, file/directory/case rename, delayed batches, drop/gap flags, owner death, cursor replacement, root replacement, restart continuity and forced-reconcile cases, and fence ordering on APFS.

- [ ] **Step 7: Commit the macOS adapter without enabling authority**

```bash
git add Cargo.toml trail/Cargo.toml trail/src/db/change_ledger/observer trail/tests/changed_path_ledger_macos.rs
git commit -m "feat: add qualified macOS change observer"
```

### Task 9: Securely auto-start one per-workspace daemon

**Files:**
- Create: `trail/src/cli/command/handler/daemon_start.rs`
- Modify: `trail/src/cli/command/handler.rs`
- Modify: `trail/src/cli/command/handler/daemon_rpc.rs`
- Modify: `trail/src/db/storage/worktree_index.rs`
- Modify: `trail/src/db/core/readonly.rs`
- Modify: `trail/src/server.rs`
- Create: `trail/tests/changed_path_ledger_daemon.rs`

**Interfaces:**
- Consumes: platform observer selection, `recover_scope`, `reconcile_full`, existing daemon RPC client/server, process liveness helpers, and executable identity.
- Produces: `ensure_workspace_daemon_ready(workspace: &Path, token: Option<&str>) -> Result<DaemonReady>`, authenticated endpoint metadata, and daemon `ledger_fence`/`ledger_reconcile` RPCs.

- [ ] **Step 1: Write concurrent startup, stale endpoint, and readiness tests**

```rust
#[test]
fn concurrent_first_status_calls_converge_on_one_owner() {
    let results = run_concurrently(16, || ensure_workspace_daemon_ready(fixture.root(), None));
    let owner_nonces = results.into_iter().map(Result::unwrap).map(|r| r.owner_nonce).collect::<HashSet<_>>();
    assert_eq!(owner_nonces.len(), 1);
}

#[test]
fn stale_endpoint_is_replaced_only_after_process_identity_mismatch() {
    fixture.write_endpoint_for_live_unrelated_process();
    assert!(ensure_workspace_daemon_ready(fixture.root(), None).is_err());
    fixture.mark_endpoint_process_dead();
    assert!(ensure_workspace_daemon_ready(fixture.root(), None).is_ok());
}
```

- [ ] **Step 2: Run the daemon tests before startup support**

Run: `cargo test -p trail --test changed_path_ledger_daemon`

Expected: FAIL because auto-discovery currently falls back instead of securely spawning and waiting for ledger readiness.

- [ ] **Step 3: Define authenticated endpoint metadata**

```rust
#[derive(Serialize, Deserialize)]
struct WorkspaceDaemonEndpoint {
    protocol_version: u16,
    pid: u32,
    process_start_identity: String,
    executable_identity: [u8; 32],
    owner_nonce: [u8; 32],
    auth_token: [u8; 32],
    socket: PathBuf,
}
```

Create endpoint/token files with owner-only permissions, publish with write+sync+atomic rename+parent-directory sync, and reject symlinks, unexpected owners/modes, wrong executable identity, PID reuse, nonce mismatch, or failed challenge-response.

- [ ] **Step 4: Implement exclusive startup convergence**

Acquire `.trail/index/change-ledger/daemon.lock`, re-check a live authenticated endpoint under the lock, spawn the current executable with an inherited one-shot readiness channel, and wait a bounded interval. Other callers wait for endpoint publication and challenge it. A live process with unverifiable identity is not killed or replaced.

- [ ] **Step 5: Make readiness include recovery, observation, reconciliation, and fence**

The daemon publishes ready only after it owns the observer lease, runs recovery, establishes native watch coverage, automatically reconciles any untrusted state, and obtains a valid live fence. Stop reading or writing `worktree-daemon-cache.json` and remove it from read-only sidecar handling; the changed-path ledger is the only persisted candidate authority.

- [ ] **Step 6: Add authenticated fence and reconcile RPCs**

```rust
pub(crate) struct DaemonReady { pub owner_nonce: [u8; 32], pub scope: ScopeId, pub cut: EvidenceCut }
pub(crate) struct FenceRequest { pub scope: ScopeId, pub expected_epoch: u64 }
pub(crate) struct ReconcileRequest { pub scope: ScopeId, pub expected: ExpectedScope, pub mode: ReconcileMode }
```

Every RPC verifies token, owner nonce, workspace identity, epoch, and executable protocol version; timeouts and broken channels fail closed.

- [ ] **Step 7: Run daemon security and lifecycle tests**

Run: `cargo test -p trail --test changed_path_ledger_daemon`

Expected: PASS for 16-way startup, PID reuse, malicious symlink/permissions, bad nonce/token, stale endpoint replacement, daemon kill/restart reconciliation, and readiness timeout.

- [ ] **Step 8: Commit daemon lifecycle**

```bash
git add trail/src/cli/command/handler.rs trail/src/cli/command/handler/daemon_rpc.rs trail/src/cli/command/handler/daemon_start.rs trail/src/db/storage/worktree_index.rs trail/src/db/core/readonly.rs trail/src/server.rs trail/tests/changed_path_ledger_daemon.rs
git commit -m "feat: auto-start authenticated workspace daemon"
```

### Task 10: Integrate one fenced snapshot flow into status, diff, and record

**Files:**
- Create: `trail/src/db/change_ledger/snapshot.rs`
- Modify: `trail/src/db/core/status.rs`
- Modify: `trail/src/db/record/diff.rs`
- Modify: `trail/src/db/record/recording/manual.rs`
- Modify: `trail/src/db/lane/control/agent_capture.rs`
- Modify: `trail/src/cli/command/handler.rs`
- Modify: `trail/src/db/performance.rs`
- Create: `trail/tests/changed_path_ledger_commands.rs`

**Interfaces:**
- Consumes: ready daemon, `ChangedPathLedger`, `QualifiedObserver`, `recover_scope`, `reconcile_full`, CAS store, and existing root point lookup/hash builders.
- Produces: `ChangedPathLedger::authoritative_snapshot`, `CandidateComparison`, and `ObservedRecordCut` used by workspace and lane record paths.

- [ ] **Step 1: Write public-flow and sequence-boundary tests**

```rust
#[test]
fn clean_status_uses_empty_authoritative_candidates_not_a_full_walk() {
    let report = fixture.status().unwrap();
    assert!(report.is_clean());
    assert_eq!(report.metrics.full_scope_walks, 0);
    assert_eq!(report.metrics.candidate_reads, 0);
}

#[test]
fn record_retains_event_arriving_between_c1_and_c2() {
    fixture.change("a", b"one");
    fixture.inject_after_first_fence(|| fixture.change("a", b"two"));
    fixture.record().unwrap();
    assert!(fixture.status().unwrap().changed_paths().contains("a"));
}
```

- [ ] **Step 2: Run command tests and observe full-scan behavior**

Run: `cargo test -p trail --test changed_path_ledger_commands`

Expected: FAIL because status/diff/record do not consume a live fenced candidate snapshot.

- [ ] **Step 3: Implement the sole authoritative snapshot entry point**

```rust
pub(crate) fn authoritative_snapshot(
    &self,
    observer: &dyn QualifiedObserver,
    expected: &ExpectedScope,
) -> Result<CandidateSnapshot> {
    self.recover(expected)?;
    let c1 = observer.fence()?;
    let durable = observer.flush_durable(&c1)?;
    self.fold_tail_through(expected, &durable)?;
    self.snapshot_candidates(expected)
}
```

If any step cannot prove trust, ask the daemon to run full reconciliation and retry once against the new epoch/cut. Only return `ChangeLedgerReconcileRequired` when automatic recovery cannot establish authority.

- [ ] **Step 4: Make status and diff candidate-only**

Load exact paths and complete prefixes, expand only affected baseline ranges/new subtrees, use pinned no-follow reads, batch root point lookups, and compare against the shared `RecordingPolicySnapshot`. Fence through `c2`; retry or retain evidence newer than `c1`. An empty authoritative set is the only fast clean result.

- [ ] **Step 5: Make manual record an observed checkpoint transaction**

Acquire workspace lock; capture ref/epoch; obtain/fold `c1`; read/hash bounded candidates; obtain/fold `c2`; build immutable target; and in one SQLite transaction index operation, CAS ref, advance baseline, and acknowledge only evidence covered through `c1`. Use this same state machine for native-agent checkpoints in `lane/control/agent_capture.rs`. Repair mirrors after commit and retain later/different-source same-path evidence.

- [ ] **Step 6: Record structural metrics**

```rust
pub(crate) struct ChangedPathMetrics {
    pub full_scope_walks: u64,
    pub root_full_ranges: u64,
    pub sqlite_full_index_rows: u64,
    pub authoritative_candidates: u64,
    pub candidate_reads: u64,
    pub hashes: u64,
    pub root_point_lookups: u64,
    pub ledger_rows_touched: u64,
    pub observer_tail_records_folded: u64,
    pub reconciliation_runs: u64,
}
```

Keep reconciliation counters separate so an O(N) cold run cannot be reported as a warm fast path.

- [ ] **Step 7: Run command correctness and locality tests**

Run: `cargo test -p trail --test changed_path_ledger_commands`

Expected: PASS for `k=0,1,100`, create/content/mode/delete/file rename/directory rename/case-only rename/revert/ignored paths, c1/c2 races, daemon restart, auto reconciliation, and full-scan oracle equality after each measured call.

- [ ] **Step 8: Commit command integration while activation remains off**

```bash
git add trail/src/db/change_ledger/snapshot.rs trail/src/db/core/status.rs trail/src/db/record/diff.rs trail/src/db/record/recording/manual.rs trail/src/db/lane/control/agent_capture.rs trail/src/cli/command/handler.rs trail/src/db/performance.rs trail/tests/changed_path_ledger_commands.rs
git commit -m "feat: consume fenced changed-path snapshots"
```

### Task 11: Cover every workspace and materialized-lane filesystem producer

**Files:**
- Modify: `trail/src/db/record/checkout.rs`
- Modify: `trail/src/db/lane/patching.rs`
- Modify: `trail/src/db/lane/workdir/materialize.rs`
- Modify: `trail/src/db/lane/workdir/sync.rs`
- Modify: `trail/src/db/lane/workdir/record.rs`
- Modify: `trail/src/db/lane/readiness.rs`
- Modify: `trail/src/model/reports/lane.rs`
- Create: `trail/tests/changed_path_ledger_producers.rs`

**Interfaces:**
- Consumes: intent lifecycle/recovery, fenced snapshots, workspace/lane scope IDs, ref transaction, and immutable target roots.
- Produces: `run_ref_advancing_projection`, `run_projection_alignment`, lane scope reconciliation, and compact `MaterializedLaneMarkerV2`.

- [ ] **Step 1: Add a checked-in producer inventory test**

```rust
const CONTROLLED_PRODUCERS: &[IntentProducer] = &[
    IntentProducer::Checkout,
    IntentProducer::LaneSync,
    IntentProducer::Materialize,
    IntentProducer::StructuredPatchProjection,
    IntentProducer::RestoreProjection,
    IntentProducer::CowPublication,
    IntentProducer::ObservedCheckpoint,
];

#[test]
fn every_filesystem_producer_has_a_reviewed_protocol() {
    assert_eq!(discover_producers_from_source(), CONTROLLED_PRODUCERS);
}
```

- [ ] **Step 2: Run producer tests and identify uncovered mutations**

Run: `cargo test -p trail --test changed_path_ledger_producers`

Expected: FAIL listing checkout, sync, materialize, structured patch, restoration, COW, or record sites without intent/fence coverage.

- [ ] **Step 3: Wrap ref-advancing projections**

```rust
fn run_ref_advancing_projection(
    db: &mut Trail,
    expected: &ExpectedScope,
    target: IntentTarget,
    changes: IntentEvidence,
    apply: impl FnOnce() -> Result<VerifiedFilesystemCut>,
) -> Result<()>;
```

Prebuild target/operation, durably prepare, apply+sync+verify, fence, then publish operation/ref/ledger in one SQL transaction. Acknowledge only intent-owned/source-covered evidence.

- [ ] **Step 4: Wrap projection-only alignment**

Checkout materialization, sparse hydration, and ordinary workdir sync reference the existing target root, durably prepare, apply+sync+verify, fence, and atomically publish only the unchanged-ref scope baseline/marker plus terminal intent. They do not fabricate an operation or advance a ref.

- [ ] **Step 5: Replace lane manifests with compact v2 markers only after reconciliation**

```rust
#[derive(Serialize, Deserialize)]
pub(crate) struct MaterializedLaneMarkerV2 {
    pub version: u16,
    pub scope_id: ScopeId,
    pub filesystem_identity: Vec<u8>,
    pub ref_name: String,
    pub ref_generation: u64,
    pub root_id: ObjectId,
    pub policy_fingerprint: [u8; 32],
    pub epoch: u64,
    pub provider_cut: EvidenceCut,
}
```

Publish marker and trusted scope identity at one logical cut. Missing, unsupported-version, or mismatched markers trigger automatic full lane reconciliation; they never prove candidates.

- [ ] **Step 6: Route lane record/readiness/merge/preview through candidates**

Materialized lane status, record, readiness, merge checks, structured-patch maintenance, and normal preview use the lane scope snapshot when trusted. Full ignored/risk traversals remain explicitly labeled audit or reconciliation operations.

- [ ] **Step 7: Run producer crash and E2E tests**

Run: `cargo test -p trail --test changed_path_ledger_producers && cargo test -p trail --test e2e lane_`

Expected: PASS for every producer, same-path external races, kill points, v2 marker mismatch, owner loss/reconcile, and post-operation oracle equality.

- [ ] **Step 8: Commit the producer inventory and integrations**

```bash
git add trail/src/db/record/checkout.rs trail/src/db/lane trail/src/model/reports/lane.rs trail/tests/changed_path_ledger_producers.rs
git commit -m "feat: cover workspace and lane producers with intents"
```

### Task 12: Make workspace-view/COW journals authoritative only when qualified

**Files:**
- Modify: `trail/src/db/lane/workdir/view_journal.rs`
- Modify: `trail/src/db/lane/workdir/view_core.rs`
- Modify: `trail/src/db/lane/workspace_view.rs`
- Modify: `trail/src/db/lane/workdir/fuse.rs`
- Modify: `trail/src/db/lane/workdir/nfs_overlay.rs`
- Create: `trail/tests/changed_path_ledger_views.rs`

**Interfaces:**
- Consumes: workspace-view scope, segment codec primitives, intent recovery, workspace lock, shared/exclusive view barrier, and checkpoint publication.
- Produces: `ViewIntentWriter`, `ViewJournalCut`, `checkpoint_view`, generation ownership, compact incremental whiteout log, and qualified view trust.

- [ ] **Step 1: Write journal-before-mutation and recovery tests**

```rust
#[test]
fn semantic_upper_mutation_is_not_visible_before_intent_is_durable() {
    let view = fixture.inject_journal_sync_failure();
    assert!(view.write("a", b"new").is_err());
    assert_eq!(view.read("a").unwrap(), b"old");
}

#[test]
fn untrusted_view_scans_upper_instead_of_claiming_zero_recovery_walks() {
    fixture.corrupt_view_journal();
    let report = fixture.checkpoint().unwrap();
    assert!(report.upper_recovery_walks > 0);
}
```

- [ ] **Step 2: Run view tests before changing journal semantics**

Run: `cargo test -p trail --test changed_path_ledger_views`

Expected: FAIL because journal/whiteout durability and scope trust are not coupled.

- [ ] **Step 3: Persist per-view intent before upper/whiteout mutation**

VFS callbacks take only the shared view barrier, append+sync the intent record, then expose the semantic upper/whiteout change. They never acquire the workspace lock or primary SQLite connection. Group commit may batch only when no mutation becomes externally visible before its intent is durable.

- [ ] **Step 4: Publish checkpoints under the required lock order**

Checkpoint takes workspace lock, then exclusive view barrier, folds the journal through a cut, verifies candidates, builds the immutable target, and publishes ref/ledger/marker in one transaction. Replay records after the clean cut into the next generation; retain active-handle ownership of older generations.

- [ ] **Step 5: Replace whole-array whiteouts with an incremental log/map**

Each whiteout mutation is atomic and replayable. Rotate at checkpoint, compact only generations with no active handle, and keep the old upper/whiteout set until recovery proves the new generation published.

- [ ] **Step 6: Run FUSE/NFS/COW crash and concurrency tests**

Run: `cargo test -p trail --test changed_path_ledger_views && cargo test -p trail db::lane::workdir::fuse db::lane::workdir::nfs_overlay`

Expected: PASS for journal sync failure, whiteout replay, active handles, checkpoint crash, generation rotation, corrupt/gapped fallback reconciliation, and `upper_recovery_walks=0` only for qualified trusted journals.

- [ ] **Step 7: Commit view correctness and compaction**

```bash
git add trail/src/db/lane/workdir/view_journal.rs trail/src/db/lane/workdir/view_core.rs trail/src/db/lane/workspace_view.rs trail/src/db/lane/workdir/fuse.rs trail/src/db/lane/workdir/nfs_overlay.rs trail/tests/changed_path_ledger_views.rs
git commit -m "feat: qualify workspace view change journals"
```

### Task 13: Keep Git authoritative only under exact Trail-baseline equivalence

**Files:**
- Modify: `trail/src/db/storage/git.rs`
- Modify: `trail/src/db/record/recording/git.rs`
- Modify: `trail/src/db/lane/workspace_git.rs`
- Modify: `trail/src/db/change_ledger/types.rs`
- Modify: `trail/src/db/performance.rs`
- Create: `trail/tests/changed_path_ledger_git.rs`

**Interfaces:**
- Consumes: Trail baseline/ref identity, `CompiledPolicy`, filesystem identity, Git mapping records, and Git command runner.
- Produces: `GitEvidenceQualification`, `qualified_git_candidates`, and Git structural metrics; it never bypasses `ChangedPathLedger::authoritative_snapshot`.

- [ ] **Step 1: Write equivalence rejection tests**

```rust
#[test]
fn git_head_match_is_insufficient_when_trail_baseline_is_ahead() {
    fixture.record_trail_ahead_of_head();
    fixture.revert_worktree_to_head();
    assert!(!fixture.qualify_git().unwrap().clean_proof_allowed);
}

#[test]
fn sparse_skip_worktree_or_policy_mismatch_is_advisory_only() {
    for mode in [GitMode::Sparse, GitMode::SkipWorktree, GitMode::PolicyMismatch] {
        assert!(!fixture.with_mode(mode).qualify_git().unwrap().clean_proof_allowed);
    }
}
```

- [ ] **Step 2: Run Git qualification tests**

Run: `cargo test -p trail --test changed_path_ledger_git`

Expected: FAIL because existing Git fast paths do not bind all semantics to the exact Trail baseline.

- [ ] **Step 3: Define the complete qualification record**

```rust
pub(crate) struct GitEvidenceQualification {
    pub head_oid: String,
    pub index_identity: Vec<u8>,
    pub mapped_trail_root: ObjectId,
    pub filesystem_identity: Vec<u8>,
    pub policy_fingerprint: [u8; 32],
    pub mode_equivalent: bool,
    pub symlink_equivalent: bool,
    pub sparse_equivalent: bool,
    pub submodule_equivalent: bool,
    pub clean_proof_allowed: bool,
}
```

Require mapped Trail baseline root exactly equal to the ledger baseline and equivalent file-mode, symlink, sparse, submodule, ignore, case, HEAD, and index/split/shared-index identities.

- [ ] **Step 4: Parse porcelain v2 `-z` conservatively**

Retain both rename endpoints. `assume-unchanged`, `skip-worktree`, unresolved sparse/submodule state, racy or untrusted fsmonitor, untrusted untracked-cache continuity, or policy mismatch forces advisory evidence. Advisory exact dirty paths may be added but cannot prove completeness or clean.

- [ ] **Step 5: Record subprocess and hidden-global-work metrics**

Capture Git subprocess count, index refresh, Trace2 regions/bytes, fsmonitor qualification, and untracked-cache qualification. Any internal global scan sets `external_adapter_global_work>0` and fails warm structural gates.

- [ ] **Step 6: Run Git oracle tests and commit**

Run: `cargo test -p trail --test changed_path_ledger_git`

Expected: PASS for exact equivalence, Trail-ahead reversion, rename endpoints, modes/symlinks, sparse/submodules, split/shared index replacement, racy fsmonitor, policy mismatch, and candidate equality with the direct Git oracle.

```bash
git add trail/src/db/storage/git.rs trail/src/db/record/recording/git.rs trail/src/db/lane/workspace_git.rs trail/src/db/change_ledger/types.rs trail/src/db/performance.rs trail/tests/changed_path_ledger_git.rs
git commit -m "feat: qualify Git changed-path evidence"
```

### Task 14: Expose reconciliation and diagnostics through every public surface

**Files:**
- Modify: `trail/src/cli/command/maintenance_args.rs`
- Modify: `trail/src/cli/command/handler/maintenance.rs`
- Modify: `trail/src/cli/command/render/maintenance.rs`
- Modify: `trail/src/cli/command/handler/errors.rs`
- Modify: `trail/src/model/reports/maintenance.rs`
- Modify: `trail/src/server/openapi/paths/core.rs`
- Modify: `trail/src/server/openapi/schemas/core.rs`
- Modify: `trail/src/server/openapi.rs`
- Modify: `trail/src/server/request_types.rs`
- Modify: `trail/src/server/route/dispatch.rs`
- Modify: `trail/src/server/route/system.rs`
- Modify: `trail/src/server/route/utils.rs`
- Modify: `trail/src/mcp/tools.rs`
- Modify: `trail/src/mcp/tools/core.rs`
- Modify: `trail/src/mcp/tool_call/core.rs`
- Modify: `trail/src/mcp/types/core.rs`
- Modify: `trail/src/mcp/response.rs`
- Modify: `trail/tests/e2e.rs`
- Create: `trail/tests/changed_path_ledger_api.rs`

**Interfaces:**
- Consumes: `ChangeLedgerReconcileReport`, stable errors, daemon reconcile RPC, workspace scope, and lane scope lookup.
- Produces: `trail index reconcile [--lane <lane>]`, structured CLI/REST/MCP errors, OpenAPI request/report schemas, and human diagnostics.

- [ ] **Step 1: Write CLI, JSON, REST, OpenAPI, and MCP contract tests**

```rust
#[test]
fn reconcile_required_has_identical_structured_recovery_fields() {
    let cli = fixture.cli_json_status_failure();
    let rest = fixture.rest_status_failure();
    let mcp = fixture.mcp_status_failure();
    for value in [cli, rest, mcp] {
        assert_eq!(value.pointer("/error/code").unwrap(), "CHANGE_LEDGER_RECONCILE_REQUIRED");
        assert_eq!(value.pointer("/error/recovery/command").unwrap(), "trail index reconcile");
    }
}
```

- [ ] **Step 2: Run surface contract tests**

Run: `cargo test -p trail --test changed_path_ledger_api`

Expected: FAIL because reconcile command/report and structured fields do not exist.

- [ ] **Step 3: Add command arguments and report type**

```rust
#[derive(Args)]
pub(super) struct IndexReconcileArgs {
    #[arg(long)]
    pub(super) lane: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChangeLedgerReconcileReport {
    pub scope_id: String,
    pub scope_kind: String,
    pub previous_state: String,
    pub reason: String,
    pub observed_paths: u64,
    pub candidates: u64,
    pub resulting_epoch: u64,
    pub resulting_state: String,
}
```

Dispatch `IndexSubcommand::Reconcile`, auto-start the daemon, resolve workspace or lane scope, run full reconciliation, and render the report in human, JSON, and NDJSON formats.

- [ ] **Step 4: Add stable diagnostic fields everywhere**

Human guidance names the scope/reason and exact command. JSON, REST, daemon, and MCP include `code`, exit/status code, `scope`, `state`, `reason`, and `recovery.command`. `SCHEMA_REINITIALIZE_REQUIRED` specifically instructs backup plus `trail init --force` and never suggests migration.

- [ ] **Step 5: Add OpenAPI route and schema checks**

Expose `POST /v1/index/reconcile` with optional lane, include the report and structured error schemas, regenerate the checked contract through the existing builder, and assert every `$ref` resolves.

- [ ] **Step 6: Run contract tests and commit**

Run: `cargo test -p trail --test changed_path_ledger_api && cargo test -p trail server::openapi mcp::`

Expected: PASS with byte-stable error codes and matching recovery fields across all surfaces.

```bash
git add trail/src/cli/command/maintenance_args.rs trail/src/cli/command/handler/maintenance.rs trail/src/cli/command/render/maintenance.rs trail/src/cli/command/handler/errors.rs trail/src/model/reports/maintenance.rs trail/src/server trail/src/mcp trail/tests/e2e.rs trail/tests/changed_path_ledger_api.rs
git commit -m "feat: expose changed-path reconciliation diagnostics"
```

### Task 15: Pass fault/scale gates, audit producers, and activate on Linux/macOS

**Files:**
- Modify: `trail/src/db/change_ledger/mod.rs`
- Modify: `trail/src/db/change_ledger/observer/mod.rs`
- Modify: `trail/src/db/performance.rs`
- Modify: `scripts/cli-scale-bench.sh`
- Create: `scripts/check-changed-path-ledger-thresholds.py`
- Create: `scripts/test_check_changed_path_ledger_thresholds.py`
- Modify: `.github/workflows/scale.yml`
- Create: `.github/workflows/changed-path-ledger-native.yml`
- Modify: `trail/tests/e2e.rs`
- Create: `trail/tests/changed_path_ledger_activation.rs`
- Modify: `docs/superpowers/specs/2026-07-12-correctness-first-changed-path-ledger-design.md`

**Interfaces:**
- Consumes: all Tasks 1–14, producer inventory, native qualification suites, structural metrics, and full-scan oracle.
- Produces: `LEDGER_AUTHORITY_ENABLED` default-on for Linux/macOS only, scheduled native jobs, 1k/100k/1M thresholds, and an auditable activation report.

- [ ] **Step 1: Write an activation test that initially fails closed**

```rust
#[test]
fn authority_requires_every_gate_and_supported_platform() {
    let complete = ActivationEvidence::from_checked_artifacts().unwrap();
    assert!(complete.schema_hard_cutover);
    assert!(complete.producer_inventory_complete);
    assert!(complete.linux_native_suite);
    assert!(complete.macos_native_suite);
    assert!(complete.crash_matrix);
    assert!(complete.scale_gates);
    assert_eq!(ledger_authority_enabled_for("windows", &complete), false);
}
```

- [ ] **Step 2: Run the activation test before flipping the default**

Run: `cargo test -p trail --test changed_path_ledger_activation`

Expected: FAIL naming every missing checked artifact or still-disabled platform default.

- [ ] **Step 3: Complete the process-kill and corruption matrix**

Exercise kill/failure before and after object write, intent sync, filesystem sync, observer sync, fence, operation index, ref CAS, ledger CAS, mirror repair, backup rotation, restore, segment rotation, and GC. Corrupt checksum, hash chain, durable/folded offsets, owner token, format version, partial SQLite shape, and root identity. Every ambiguous outcome may be a false-positive candidate or reconciliation, never false clean.

- [ ] **Step 4: Add 1k, 100k, and 1M benchmark modes with separate `k`**

Run workspace status/diff/record, materialized-lane record, structured-patch maintenance, and COW checkpoint for authoritative input `k=0,1,100`, independently recording final changed output. CI runs 1k; scheduled ext4 and APFS jobs run 100k; nightly jobs run 1M plus cold reconciliation, daemon restart, and crash sampling.

- [ ] **Step 5: Enforce structural fast-path bounds**

```python
ZERO = [
    "full_scope_walks", "root_full_ranges", "sqlite_full_index_rows",
    "full_manifest_bytes_read", "full_manifest_bytes_written",
    "upper_recovery_walks", "external_adapter_global_work",
    "policy_dependency_full_discovery",
]
for key in ZERO:
    assert metrics[key] == 0, f"{key} must remain zero on a warm trusted run"
assert metrics["candidate_reads"] <= metrics["authoritative_candidates"] + metrics["bounded_prefix_output"]
assert metrics["observer_tail_records_folded"] <= metrics["configured_tail_bound"]
assert metrics["ledger_rows_touched"] <= 8 * (metrics["authoritative_candidates"] + metrics["bounded_prefix_output"] + 1)
```

Also enforce persisted caps for candidate/prefix rows and log/segment bytes, plus calibrated wall-time and peak-RSS budgets per host class. Label all reconciliation work separately.

- [ ] **Step 6: Run the full-scan oracle outside every measured fast path**

After each warm command completes and metrics are frozen, run direct full observation and assert exact semantic equality. Property-test event permutations, prefix coalescing, rename ambiguity, callback delays, concurrent same-path writes, and acknowledgement boundaries.

- [ ] **Step 7: Run platform and repository gates**

Run on Linux: `cargo test -p trail --test changed_path_ledger_linux -- --nocapture && REPO_FILES=100000 scripts/cli-scale-bench.sh changed-path-ledger`

Run on macOS: `cargo test -p trail --test changed_path_ledger_macos -- --nocapture && REPO_FILES=100000 scripts/cli-scale-bench.sh changed-path-ledger`

Run everywhere: `cargo fmt --all -- --check && cargo check --workspace --all-targets && cargo test --workspace`

Expected: all commands PASS; scheduled outputs satisfy structural, time, and RSS thresholds. Other platforms pass direct-observation tests and assert that no scope row can transition to `trusted`.

- [ ] **Step 8: Flip authority only after checked evidence exists**

```rust
#[cfg(any(target_os = "linux", target_os = "macos"))]
pub(crate) const LEDGER_AUTHORITY_ENABLED: bool = true;
#[cfg(not(any(target_os = "linux", target_os = "macos")))]
pub(crate) const LEDGER_AUTHORITY_ENABLED: bool = false;
```

Keep runtime qualification mandatory even on supported operating systems: unsupported filesystems/providers automatically reconcile and, if qualification remains impossible, return the stable recovery error rather than persisting trust.

- [ ] **Step 9: Record the activation audit and commit**

Append an “Activation evidence” section to the approved design with exact workflow names, artifact paths, producer inventory hash, native adapter results, scale thresholds, and full-workspace test command. Then commit the activation and gates together.

```bash
git add trail/src/db/change_ledger trail/src/db/performance.rs trail/tests scripts .github/workflows docs/superpowers/specs/2026-07-12-correctness-first-changed-path-ledger-design.md
git commit -m "feat: activate changed-path ledger on Linux and macOS"
```

---

## Final Verification and Integration

- [ ] Run `cargo fmt --all -- --check` and expect exit status 0.
- [ ] Run `cargo check --workspace --all-targets` and expect exit status 0.
- [ ] Run `cargo test --workspace` and expect every unit, integration, E2E, property, and contract test to pass.
- [ ] Run `git diff --check` and expect no whitespace errors.
- [ ] Run `rg -n "legacy_reconcile_required|migrate_changed_path|backfill_changed_path|CREATE IF NOT EXISTS.*changed_path" trail/src trail/tests` and expect no matches.
- [ ] Run `rg -n "worktree-daemon-cache.json" trail/src` and expect no matches.
- [ ] On Linux and macOS, run the native observer and 100k commands from Task 15 and archive their machine-readable reports.
- [ ] Open the same 100k fixture on an unsupported platform build and prove it uses direct observation and persists no trusted scope.
- [ ] Inspect `git status --short`, exclude `.superpowers/` scratch state, and ensure every intended source/test/doc file is committed.
- [ ] Merge the completed feature branch into local `main` only after every activation check above passes; do not force, prune, rewrite unrelated work, or include unrelated dirty files.
