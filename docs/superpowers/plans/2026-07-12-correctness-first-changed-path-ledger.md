# Changed-Path Ledger Implementation Plan

Status: approved implementation plan

Design: `docs/superpowers/specs/2026-07-12-correctness-first-changed-path-ledger-design.md`

Every task is test-first, independently reviewed, and committed separately.
The ledger remains dormant/feature-disabled through Tasks 2–6. No scope may
enter `trusted`, and no public read path may consume ledger clean proof, until
recovery, policy dependencies, every controlled producer, and a qualified
external adapter are complete.

## Task 1: Structural metrics and true selected-path locality

- Add operation-scoped counters for filesystem walks, root ranges, SQLite rows,
  input candidates versus final changes, hashes, point lookups, manifest/log
  bytes, journal/upper work, Git global work, wall time, and peak RSS.
- Replace `prune_worktree_index_for_selections` global enumeration with indexed
  exact/binary-prefix queries.
- Replace repeated selected-map filtering and other `O(k^2)` loops with bounded
  unions/batches; batch Prolly reads and index updates.
- Gate existing daemon/Git selected status, dirty diff, and record at 1k with
  zero hidden global rows. This task changes locality, not trust semantics.

## Task 2: Dormant next-schema migration and deep ledger core

- Add scope, exact-path, prefix, intent path/prefix, reconciliation staging,
  and observer segment/lease tables with required indexes/foreign keys.
- Bump the implementation-time schema by one (v16→v17 or v17→v18 if the
  path-index recovery migration lands first). Verify exact new-version tables,
  columns, constraints, indexes, schema metadata, and `PRAGMA user_version`
  even on the startup fast path.
- Implement typed scope/filesystem identity, capabilities, trust states,
  source-sequenced evidence, coalescing, intent lifecycle, CAS, and corruption
  handling behind a disabled feature gate.
- New/migrated scopes start `legacy_reconcile_required`; `begin_scope` cannot
  produce trust.
- Add schema/log-format version, partial migration, stale owner, root/policy/
  filesystem replacement, and concurrent CAS tests.

## Task 3: Reconciliation, diagnostics, recovery, backup, and GC

- Implement crash-safe streamed reconciliation staging with full baseline-root
  deletion comparison, pinned no-follow reads, full hashing after gaps, and
  before/after identity checks.
- Publish only under ref/root/scope/policy/filesystem/provider-fence CAS.
- Permit prefix promotion only for provider-proven complete invalidation;
  global gaps always require full scope reconciliation.
- Add stable `CHANGE_LEDGER_RECONCILE_REQUIRED` across human/JSON CLI, daemon
  RPC, REST/OpenAPI, and MCP, plus `trail index reconcile` reports.
- Run intent recovery before mutation/GC. Make prepared/published targets GC
  roots; specify lane/view retirement, backup fencing/untrusted snapshot, and
  restore epoch rotation/untrusted state.
- Keep the ledger disabled after this task.

## Task 4: Trail policy snapshot and dependency tracking

- Build Trail's authoritative compiled policy for its walkers/candidate checks.
- Persist the dependency manifest for nested ignore files, Git info/global/
  included config, built-in versions, normalization, modes, and case policy.
- Make adapters declare equivalent versus conservative semantics; unresolved or
  unobservable dependencies stale the scope.
- Process raw policy-file events before filtering and avoid full dependency
  rediscovery on each fast request.
- Keep all scopes untrusted/disabled.

## Task 5: Controlled mutation intents and atomic publication

- Inventory every filesystem producer and assign it to one reviewed class:
  observed record/checkpoint, ref-advancing controlled projection,
  projection-only alignment, or VFS/COW.
- For ref-advancing projection: prebuild immutable target/operation,
  durable-prepare exact/prefix intent, mutate+sync+verify+fence, then atomically
  publish operation/ref/ledger.
- For projection-only alignment (checkout materialization, sparse hydration,
  ordinary sync): reference the existing target, mutate+sync+verify, then
  fence/flush the qualified provider, retain unrelated/later evidence, and
  publish only scope baseline/marker and terminal intent under unchanged
  ref/epoch/provider-cut CAS—do not fabricate an operation or ref advance.
- Acknowledge only intent-owned/source-sequenced evidence; retain concurrent
  same-path watcher events.
- Reconcile ref/manifest mirrors from authoritative SQL and exhaust crash points
  around object, intent fsync, filesystem, verification, ref, ledger, and
  mirror transitions.
- Install recovery before allowing another mutation. Ledger remains disabled as
  read authority until every controlled producer is covered.

## Task 6: Qualified observer log and Git adapters

- Implement versioned segments with exclusive epoch/owner lease, monotonic
  sequence, checksum/hash chain, durable/folded offsets, fsync/group-commit,
  rotation, bounded tail, disk-full/error revocation, and read-only safe-tail
  semantics.
- Define/test provider capability matrix and real delivery fence/cursor
  contracts. Generic callbacks never qualify clean proof without quiescence.
- Cover delayed delivery, backlog, cursor discontinuity, owner death/restart,
  lease replacement, append failure, torn tail, rotation crash, and power-loss
  claims.
- Implement Git porcelain-v2 adapter only when ledger baseline exactly equals
  mapped HEAD/index tree and sparse/submodule/mode/policy/fsmonitor semantics
  qualify; otherwise advisory only. Measure Trace2/index work.
- Ledger remains disabled until Task 7 activation audit.

## Task 7: Activate main workspace trusted reads

- Run an activation audit proving Tasks 2–6 cover every workspace filesystem
  producer and recovery path; feature gate defaults off until this passes.
- Integrate trusted snapshots into status, read-only status, dirty diff, manual
  record, and native-agent checkpoints.
- Implement the observed record/checkpoint state machine for manual record and
  native-agent checkpoint: fence c1, read/hash, fence c2, build target, then
  atomically publish ref/ledger while acknowledging only c1-covered evidence.
- Snapshot consumers fold/merge durable tails or return reconcile-required;
  unread evidence can never be ignored.
- Preserve compatibility: interactive status may perform labeled automatic
  reconciliation; unapproved expensive mutations fail with recovery guidance.
- Gate warm `k=0,1,100`, cold start, owner restart, and gap behavior separately.

## Task 8: Materialized lane v2 marker and deployment contract

- Create per-lane scopes owned by qualified long-lived observers.
- Publish compact v2 marker and trusted SQL scope identity at one logical cut
  only after explicit reconciliation; legacy manifests cannot prove candidates.
- CLI-only/no-qualified-observer lanes become `untrusted_gap` on process exit.
- Make lane status, record, readiness, merge checks, structured-patch workdir
  maintenance, and ordinary preview candidate-only only in trusted scopes.
- Keep full ignored/risk traversal as explicit audit/reconciliation.

## Task 9: COW journal correctness, then compaction

- VFS callbacks use shared view barrier plus per-view fsynced intent log only;
  never workspace lock/primary SQLite. Checkpoint uses workspace lock then
  exclusive barrier, folds/verifies, and publishes.
- First prove journal/whiteout atomicity, group commit, active-handle generation
  ownership, recovery, and crash behavior.
- Then rotate/compact at checkpoint, roll generations when handles permit, and
  replace whole-array whiteouts with an incremental map/log.
- Assert zero upper scan only for qualified trusted journals; gaps explicitly
  reconcile.

## Task 10: Cross-platform scale, fault, and production gates

- CI 1k; scheduled 100k/1M on actual Linux/macOS/Windows filesystem/provider
  tiers. Publish capability matrix with unsupported/network filesystems failing
  closed.
- Exercise input candidates `k=0,1,100` separately from final changed output
  across workspace status/diff/record, materialized lanes, structured patch,
  and COW checkpoint.
- Cover create/content/mode/delete, file/directory/case rename, reverted events,
  nested/global policy, delayed delivery/backlog, overflow, disk full, daemon
  restart, epoch replacement, concurrent same-path writes, migration, backup/
  restore/GC, and kill points at every fsync/publication boundary.
- Warm trusted gates require zero full walks/ranges/global-index rows, zero
  legacy manifest I/O, zero upper recovery, and reads/rows proportional to
  authoritative candidates plus unavoidable prefix output.
- Warm gates also require zero external-adapter global work and policy full
  discovery, bounded folded tail/log bytes/rows, and calibrated wall-time/RSS
  budgets. Persisted caps fail closed when exceeded.
- Cold/gapped O(N) work must be labeled reconciliation. Run a full-scan oracle
  after measured fast paths and property-test event permutations for no false
  clean.
- Run fmt/check/full tests/docs/OpenAPI/backup plus independent architecture,
  security, crash-consistency, and performance review before default enabling.
