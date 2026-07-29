# Complete Lane Environment Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make lane-private framework artifacts disposable, inherit compatible immutable environment state on forks, and route every execution surface through one environment lifecycle.

**Architecture:** Schema v21 adds a replayable lane-retirement journal so runtime, mount, observer, database, and filesystem cleanup converges across crashes. Forks snapshot compatible immutable generation records and layer IDs into a child generation while retaining fresh private uppers. A managed-execution module centralizes preparation and finalization for lane commands, agents, and ACP.

**Tech Stack:** Rust, rusqlite/SQLite, serde, clap, Trail HTTP/MCP, macOS NFS-COW, Docker/Podman runtime ownership.

## Global Constraints

- Preserve all pre-existing uncommitted changes and the current observer-retirement behavior.
- Write every behavioral test first and observe the expected failure before production edits.
- Never inherit source, generated, scratch, runtime, secret, lease, sync-attempt, or mutable cache state.
- Never delete a filesystem path unless it is revalidated as confined to the lane workdir or `.trail` private storage.
- Never reclaim an immutable layer while any active, archived, or retained generation references it.
- Preserve the primary command result while reporting every checkpoint, cleanup, and unmount failure.
- No schema-v21 database may be partially opened by a v20 binary.

---

### Task 1: Schema v21 retirement journal

**Files:**
- Modify: `trail/src/db/mod.rs`
- Modify: `trail/src/db/storage/schema/ddl.rs`
- Modify: `trail/src/db/storage/schema.rs`
- Modify: `trail/src/db/storage/mod.rs`
- Modify: `trail/src/db/core/init.rs`
- Create: `trail/tests/schema_v21_lane_retirements.rs`

**Interfaces:**
- Produces: `LANE_RETIREMENTS_V21`, `validate_schema_v21`, and `migrate_schema_v20_to_v21`.

- [ ] Write a v20 fixture migration test asserting the exact `lane_retirements` table, indexes, phase/kind checks, rollback at the injected boundary, backup/restore, and downgrade refusal.
- [ ] Run `cargo test -p trail --test schema_v21_lane_retirements -- --nocapture`; verify RED because schema v21 is absent.
- [ ] Add the table and exact validation/migration entry points, update `TRAIL_SCHEMA_VERSION` to `21`, and route open/init through v21.
- [ ] Re-run the focused schema test and existing v19/v20 migration tests; verify GREEN.

### Task 2: Retirement domain model and read API

**Files:**
- Modify: `trail/src/model/lane/coordination.rs`
- Modify: `trail/src/model/mod.rs`
- Create: `trail/src/db/lane/retirement.rs`
- Modify: `trail/src/db/lane/mod.rs`
- Test: `trail/tests/lane_retirement.rs`

**Interfaces:**
- Produces: `LaneRetirementKind`, `LaneRetirementPhase`, `LaneRetirementReport`, `Trail::lane_retirement`.

- [ ] Write report serialization and lookup tests with literal phase/kind/provenance values.
- [ ] Run the focused tests; verify RED on missing types/API.
- [ ] Add the serialized models and exact row decoder/read APIs.
- [ ] Re-run the focused tests; verify GREEN.

### Task 3: Archive and unarchive

**Files:**
- Modify: `trail/src/db/lane/workdir/lifecycle.rs`
- Modify: `trail/src/db/lane/identity.rs`
- Modify: `trail/src/cli/command/lane_args.rs`
- Modify: `trail/src/cli/command/handler/lane.rs`
- Modify: `trail/src/cli/command/render/lane/work.rs`
- Modify: `trail/src/server/route/lane/lanes.rs`
- Modify: `trail/src/mcp/types/lane.rs`
- Modify: `trail/src/mcp/tool_call/lane.rs`
- Test: `trail/tests/lane_retirement.rs`
- Test: `trail/tests/e2e.rs`

**Interfaces:**
- Produces: `Trail::archive_lane`, `Trail::unarchive_lane`, CLI `lane archive|unarchive`.

- [ ] Write DB and CLI tests proving archive retains ref/view/generation/uppers, blocks execution, and unarchive restores the same identities.
- [ ] Run focused tests; verify RED on missing commands.
- [ ] Implement transactional status/event changes and all public surfaces.
- [ ] Re-run focused tests; verify GREEN.

### Task 4: Removal state machine

**Files:**
- Modify: `trail/src/db/lane/retirement.rs`
- Modify: `trail/src/db/lane/workdir/lifecycle.rs`
- Modify: `trail/src/db/lane/workspace_runtime.rs`
- Modify: `trail/src/db/lane/workspace_layer.rs`
- Test: `trail/tests/lane_retirement.rs`

**Interfaces:**
- Consumes: existing changed-path `retire_deletion_scopes` and runtime ownership validation.
- Produces: `prepare_lane_retirement`, `resume_lane_retirement`, `recover_lane_retirements`.

- [ ] Write one failing test per phase: prepared record, runtime stop, pointer/generation retirement, binding removal, confined private deletion, tombstone completion, and retry idempotence.
- [ ] For each test, run it and verify the failure names the missing phase side effect.
- [ ] Implement the minimal phase transition with compare-and-set phase updates and structured repair errors.
- [ ] Re-run after every phase, then run the full `lane_retirement` test target.

### Task 5: Crash recovery, purge, and GC

**Files:**
- Modify: `trail/src/db/lane/retirement.rs`
- Modify: `trail/src/db/core/init.rs`
- Modify: CLI/HTTP/MCP lane surfaces from Task 3
- Test: `trail/tests/lane_retirement.rs`
- Test: `trail/src/db/lane/workspace_layer.rs`

**Interfaces:**
- Produces: `Trail::purge_lane`, CLI `lane purge`, open-time recovery.

- [ ] Add subprocess crash tests at every retirement boundary and prove reopen/retry convergence.
- [ ] Add purge tests requiring force/exact identity and proving remaining tombstone/provenance deletion.
- [ ] Add unique-layer and shared-layer GC tests after completed removal.
- [ ] Implement recovery registration, purge, and reference cleanup; run all focused tests GREEN.

### Task 6: Fork environment inheritance

**Files:**
- Modify: `trail/src/db/lane/lifecycle.rs`
- Modify: `trail/src/db/lane/workspace_layer.rs`
- Modify: `trail/src/model/reports/lane.rs`
- Test: `trail/src/db/lane/workspace_layer.rs`
- Test: `trail/tests/lane_initialization.rs`

**Interfaces:**
- Produces: `inherit_workspace_environment_generation(parent_lane, child_lane)` and per-component inheritance reports.

- [ ] Write a real-layer test proving identical parent/child layer IDs and distinct source/generated/scratch paths.
- [ ] Run it; verify RED because the child has no generation.
- [ ] Implement compatibility filtering and child generation/component/output/cache provenance copying, excluding runtime/secrets/leases/sync state.
- [ ] Add mixed compatibility, absent generation, corrupted layer, concurrent fork, and parent removal tests; iterate RED/GREEN.

### Task 7: Managed execution core

**Files:**
- Create: `trail/src/execution/mod.rs`
- Create: `trail/src/execution/managed.rs`
- Modify: `trail/src/lib.rs`
- Modify: `trail/src/db/lane/workspace_view.rs`
- Test: `trail/tests/managed_execution.rs`

**Interfaces:**
- Produces: `ManagedExecutionRequest`, `ManagedExecutionReport`, ordered preparation/finalization receipts.

- [ ] Write orchestration tests for successful ordering, preparation rollback, nonzero exit, checkpoint failure, cleanup failure, and aggregate reporting.
- [ ] Run focused tests; verify RED because the orchestration API is absent.
- [ ] Implement discover/plan, sync-all, runtime reconcile, mount, execute, source checkpoint, disposal, and unmount with RAII finalization.
- [ ] Re-run focused tests; verify GREEN.

### Task 8: Migrate execution surfaces

**Files:**
- Modify: `trail/src/cli/command/handler/lane.rs`
- Modify: `trail/src/cli/command/handler/agent.rs`
- Modify: ACP execution modules under `trail/src`
- Test: `trail/tests/e2e.rs`
- Test: ACP and agent integration targets

**Interfaces:**
- Consumes: managed execution API from Task 7.

- [ ] Add one failing integration test per surface: lane exec, test, eval, terminal agent, and ACP.
- [ ] Migrate each surface independently and run its focused test GREEN before the next.
- [ ] Add interruption tests proving finalization and source-only checkpoint behavior.

### Task 9: Documentation and completion audit

**Files:**
- Modify: `docs/reference/cli/lanes-and-workflows.md`
- Modify: `docs/design/environment-adapter-contract.md`
- Modify: OpenSpec tasks and specs under `openspec/changes/complete-lane-environment-lifecycle`

- [ ] Document archive/remove/purge, inheritance, managed execution, repair, and observable reports.
- [ ] Run `cargo test -p trail --lib`, schema/integration targets, and `cargo test -p trail --test e2e`.
- [ ] Run macOS real NFS-COW Node, Cargo, Next.js/Vite, removal/GC, and fork-reuse probes.
- [ ] Map every OpenSpec scenario to direct passing evidence and leave no requirement inferred from a narrower test.
