# Trail Production Hardening Design

**Date:** 2026-07-18

**Status:** Approved architecture and written specification

**Scope:** Trail lane initialization, observer coordination, recovery, native-COW observability, build reproducibility, and production release gates for large repositories.

## Context

The Superset verification exercised eight APFS native-COW lanes over a 10,241-file, 281,636,494-byte Trail root. Trail preserved the intended 400-file aggregate after correctness fixes, but the run and the full library suite exposed remaining production blockers:

- concurrent materialized spawns can commit a lane and then return `DAEMON_UNAVAILABLE` while independently starting materialized-lane observer authority;
- retrying a committed spawn is ambiguous because lane names reject duplicates without proving whether the duplicate request is identical or resumable;
- workspace journal recovery can attempt to rotate an older SQLite checkpoint over a newer dirty journal tail;
- nine library failures reproduce in isolation and five more appear under parallel test execution;
- native-COW lanes have no useful space report;
- the checked producer inventory digest is stale, disabling activation evidence;
- the pinned prolly submodule commit is unavailable from its configured remote;
- the large-repository workflow is evidenced by one manual run rather than a repeatable release gate.

“Production ready” in this design means the explicit acceptance gates below pass on supported platforms. It does not mean that software can be proven free of every possible defect.

## Goals

1. Make lane spawn transactionally honest and safely resumable across CLI retries, lost responses, daemon failures, and process crashes.
2. Allow at least 64 simultaneous materialized lane spawns without ambiguous outcomes or observer split-brain.
3. Preserve fail-closed observer authority, filesystem identity, and exact request identity checks.
4. Eliminate all known library test failures, including parallel-only interference.
5. Make recovery idempotent when SQLite, journal mirrors, and dirty journal tails are observed at different crash cuts.
6. Provide honest native-COW logical and allocated-space reporting without claiming physical sharing that the platform cannot prove.
7. Restore reproducible dependency and activation evidence.
8. Turn the Superset-class workload and fault matrix into blocking release automation.

## Non-goals

- Serializing complete COW materialization through the current single-threaded workspace daemon.
- Claiming exact APFS shared-extent bytes unless a supported authenticated platform API provides them.
- Weakening live-owner, PID-reuse, executable-identity, filesystem-identity, or policy-drift checks to improve availability.
- Automatically merging conflicting lanes or bypassing readiness gates.
- Supporting unlimited agents without measured resource ceilings.

## Architecture

### 1. Durable lane initialization state machine

Schema version 19 adds a `lane_initializations` table. One row represents one normalized spawn request and is the source of truth for retry behavior.

Required columns:

- `initialization_id`: content-addressed identifier derived from workspace identity, lane name, and normalized request fingerprint;
- `lane_name` and `lane_id`;
- `request_fingerprint`: SHA-256 of canonical source ref/change/root, requested workdir mode, explicit destination, normalized sparse paths, neighbor policy, provider, and model;
- `operation_id`: the materialization operation identifier when one exists, otherwise the source operation identifier;
- `phase`: `reserved`, `materialized`, `associated`, `observer_ready`, or `repair_required`;
- `workdir` and materialization metadata;
- `last_error_code`, `last_error_message`, and `repair_command`;
- `created_at` and `updated_at`.

The table enforces one active initialization per lane name. Phase transitions use compare-and-set predicates inside the same SQLite transaction as the state they describe.

State invariants:

- `reserved` means no lane ref or lane branch is externally visible.
- `materialized` means the staged or published workdir and its materialization journal are durable, but the lane association may not yet be visible.
- `associated` means the lane ref, lane row, branch row, ref mirror repair, and materialization completion are committed. Returning an ordinary “spawn failed” error after this phase is forbidden.
- `observer_ready` means initial reconciliation, marker publication, and `lane_spawned` event publication completed.
- `repair_required` means `associated` state remains usable and the exact recovery command is durable. It never means the lane was rolled back.

The migration backfills existing lanes as `observer_ready` only when their ref, branch, workdir metadata, marker/observer evidence, and spawn event are consistent. Otherwise it records `repair_required` with `trail lane repair-initialization <lane>`.

### 2. Request identity and idempotent retry

The CLI canonicalizes spawn arguments before reservation. The source is resolved to immutable change/root identifiers; path selections are normalized, sorted, and deduplicated. The canonical representation is versioned so future fields cannot silently alias an older request.

Retry rules:

- no existing row: reserve and continue;
- same lane name and same fingerprint in a nonterminal phase: resume from the durable phase;
- same lane name and same fingerprint in `observer_ready`: return the original `LaneSpawnReport` with `resumed: true`;
- same lane name and a different fingerprint: return a stable `LANE_INITIALIZATION_CONFLICT` error containing both fingerprints and no mutation;
- `repair_required`: attempt the recorded repair once under the same initialization identity, then either reach `observer_ready` or return a structured committed repair response.

The lane name plus request fingerprint is the default idempotency identity, so existing agents need no new required flag. An optional explicit request ID may be added later but is not required by this design.

### 3. Concurrent materialization and lock admission

Large file cloning remains outside the workspace write lock and may run concurrently in separate CLI processes. Only durable reservation, association, and observer-authority transitions serialize.

Workspace lock publication becomes a backward-compatible versioned record containing:

- owner PID, process-start identity, and nonce;
- purpose: `command_mutation`, `lane_association`, `observer_startup`, `observer_publication`, `schema_transition`, or `maintenance`;
- optional initialization/operation identifier;
- creation time.

Internal contenders use condition-based bounded waiting with exponential backoff and jitter. Waiting is authorized only after authenticating the existing lock record and live process identity. Unknown, malformed, replaced, stale-but-unreapable, or incompatible owners remain terminal.

Observer startup for distinct materialized-lane scopes may scan concurrently, but its short segment-owner and SQLite publication cuts serialize through the authenticated workspace lock. Same-scope acquisition remains exclusive and requires exact epoch/owner replacement authority.

The blocking admission deadline is configurable with a conservative default and an upper bound. Timeout errors include holder purpose, age, initialization identifier when present, and a safe retry command.

### 4. Transactionally honest responses and repair

Every spawn result identifies:

- `initialization_id`;
- `request_fingerprint`;
- `phase`;
- `committed`;
- `resumed`;
- lane and workdir metadata.

Failures before `associated` use ordinary rollback/error semantics. Failures at or after `associated` use `CommittedRepairRequired` with stable structured fields and a durable recovery command. JSON and human renderers must not label these outcomes as an absent lane.

`trail lane repair-initialization <lane>` is idempotent. It validates the request fingerprint, lane association, materialization journal, filesystem identity, and current head before resuming reconciliation, marker repair, and event publication. It never recreates materialized bytes unless the durable phase proves association did not occur.

### 5. Journal checkpoint recovery

SQLite checkpoint state describes the last committed clean cut; the journal may legitimately contain a newer dirty tail.

Recovery follows these rules:

- SQLite checkpoint ahead of the durable journal remains corruption.
- Exact SQLite/journal generation and sequence is idempotently accepted.
- When the journal generation matches but its last sequence is newer than the SQLite checkpoint, recovery repairs the clean-checkpoint mirror and barrier at the SQLite cut without rotating or discarding the dirty tail.
- Rotation occurs only when the durable journal last sequence equals the committed checkpoint sequence and the requested generation advances exactly once.
- A newer journal generation with an authenticated matching base cut is accepted idempotently; contradictory base hashes or skipped generations fail closed.

Tests cover zero-sequence initial views, dirty generated/private outputs, repeated reopen, crash after SQLite publication, crash after mirror publication, crash after rotation, and two views sharing one generated layer.

### 6. Known test failures and isolation

The deterministic failures are fixed at their root causes:

- producer inventory constants and activation audit are regenerated only after the producer audit test proves the checked fixture;
- empty `GIT_CONFIG` is retained as an environment-selector dependency but is not interpreted as the workspace directory file;
- persistent policy churn preserves the structured reconciliation error instead of wrapping it as generic daemon unavailability;
- crash helpers retain bounded stderr and phase diagnostics so pre-handshake failures are actionable, then the underlying filesystem-applied failure is fixed;
- the 100-open singleflight path is profiled and optimized without relaxing schema authority checks;
- journal recovery follows the dirty-tail rules above.

Parallel-only failures are treated as shared-state defects, not quarantined by globally serializing the suite. Test hooks, daemon registries, workspace locks, temporary runtime publications, and environment overrides must be keyed by workspace/test identity and cleaned by RAII. Tests that intentionally share a process-global resource must prove isolation or use an explicit scoped test guard.

### 7. Native-COW space observability

`trail lane space` supports native-COW workdirs and reports:

- logical bytes and file count;
- filesystem allocated blocks/bytes;
- bytes changed since the lane baseline when Trail can derive them from content identity;
- materialization backend and clone count;
- `physical_sharing`: `verified`, `not_shared`, or `unknown`;
- the platform evidence source.

On APFS, allocated block counts are not presented as exclusive bytes. If Trail cannot authenticate shared extents, it returns `unknown` with a reason instead of failing or inventing savings.

### 8. Reproducible dependency and schema migration

The prolly gitlink is updated to an available reviewed commit only after Trail’s focused storage, root-map, diff, merge, GC, and recovery suites pass against it. `Cargo.lock` is regenerated from that exact gitlink. The remote must retain the pinned commit, and a clean recursive clone/build is a release gate.

Schema 19 migration is transactional, idempotent, backed up by the existing schema-exclusion protocol, and tested from a real schema-18 fixture. Older binaries must fail with explicit upgrade incompatibility rather than partially opening schema 19.

## Data flow

1. CLI resolves and canonicalizes the spawn request.
2. Trail reserves or resumes `lane_initializations` by fingerprint.
3. Materialization runs concurrently outside the workspace lock and publishes through its existing staged journal.
4. A short association transaction commits the lane ref, lane, branch, and initialization phase.
5. Materialized-lane observer startup authenticates a compatible lock holder, waits boundedly, and establishes scope-specific authority.
6. Reconciliation and projection alignment prove the workdir matches the lane head.
7. Marker and event repairs complete, then the initialization becomes `observer_ready`.
8. A lost response is recovered by repeating the same command; Trail returns or resumes the same initialization.

## Error handling and crash safety

- Every filesystem publication retains the existing staged-materialization recovery journal.
- Phase updates never get ahead of the durable artifact they describe.
- Repair validates immutable source/root/request identity at every boundary.
- PID reuse and executable replacement remain fail-closed.
- Lock timeouts do not revoke live owners.
- Daemon loss after association produces a committed repair response.
- Client response loss after success produces an idempotent replay response.
- Disk-full, permission, and fsync failures preserve the last durable phase and cleanup only artifacts proven unassociated.
- Conflict handling and Git export safety remain independent gates after lane initialization.

## Verification and release gates

### Correctness gates

- `cargo test -p trail --lib` passes with zero failures under default parallelism and with `RUST_TEST_THREADS=1`.
- All integration tests pass on macOS and Linux native changed-path runners.
- Schema 18→19 migration, downgrade refusal, backup/restore, crash matrix, corruption matrix, and producer inventory gates pass.
- Focused red-green tests prove every new state transition, retry, lock timeout, dirty-tail recovery, and error payload.

### Scale gates

Blocking Superset-class gate:

- 64 simultaneous native-COW lanes;
- at least 50 disjoint edits per lane;
- concurrent record and merge-queue handoff to one Trail main;
- one safe range-delta Git export;
- zero ambiguous CLI outcomes, false deletions, missing lanes, unintended Git paths, integrity errors, or leaked live locks;
- p50/p95/p99 timing, RSS, database growth, observer-log growth, logical/allocated lane space, and retry counts recorded.

Stress evidence:

- 128 simultaneous lanes with the same invariants;
- failures may exceed the blocking latency target, but correctness and recoverability may not fail.

Fault injection includes process kill at every initialization phase, daemon death, response loss after association and after readiness, PID reuse simulation, lock-holder crash, policy churn, filesystem replacement, disk-full simulation, conflicting lanes, and dirty Git export refusal.

### Operational gates

- `trail doctor`, `trail fsck`, and Git integrity checks pass after workload and recovery.
- No stale mount, socket, lock, initialization, or materialization publication survives successful cleanup.
- Metrics and error codes are documented and stable.
- A clean recursive checkout builds using only retained pinned dependencies.

## Rollout

1. Land schema-19 migration and initialization read APIs behind inactive behavior.
2. Add idempotent reservation/resume and structured response tests.
3. Add compatible lock admission and concurrent observer tests.
4. Switch lane spawn to the state machine.
5. Fix journal recovery and all known deterministic tests.
6. Eliminate parallel-only interference and require two execution modes.
7. Add native-COW space reporting and repeatable benchmark tooling.
8. Run 64-lane gate, then 128-lane stress and fault matrices.
9. Enable release only when every blocking gate is green on the exact commit.

No phase is declared production-ready based solely on focused tests or one successful benchmark run.
