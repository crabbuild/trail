# Changelog

All notable changes to Trail are documented in this file. Trail follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Added Rust library artifact resolution component/batch operations with durable fenced
  attempts, content-addressed snapshot reuse, explicit-only refresh, bounded redacted
  evidence, and deterministic reports.
- Workspace-layer singleflight now records durable generation-fenced owner phases and
  waiter outcomes, and only recovers a lock when the exact PID/start identity is proven
  dead or mismatched.
- Workspace open now recovers dead artifact constructors and exact owned staging;
  doctor and fsck validate raw CAS objects, snapshots, envelopes, attempt coherence,
  legacy/CAS layouts, and orphan materializations with repair guidance.
- Backup/restore validation now treats omitted materialization caches as disposable,
  rebases restored layer paths before publication, and parallel environment builders
  use a bounded SQLite wait during short WAL publication overlap.
- Environment discovery now reports marker-recognized plugins that do not support the
  current host as typed `unsupported` proposals without launching plugin code.
- Native lane views now resolve verified immutable artifact manifests lazily, read only
  requested blob/chunk ranges, and materialize only touched files during copy-up while
  preserving shared FUSE, NFS, and Dokan upper/whiteout semantics.
- Real-directory artifact consumers now reuse tree-root/backend-keyed verified
  materialization caches that rebuild from authoritative CAS, restore immutable
  permissions on reuse, and clone/reflink or independently copy into mutable state.
- Lane forks now inherit only individually verified CAS-backed outputs after desired-key,
  envelope/tree, current adapter package, scope, portability, and backend checks, while
  allocating fresh artifact bindings and private workspace identities.
- Portable backups now retain source uppers and authoritative artifact snapshots,
  objects, envelopes, attestations, historical generations, and exact bindings while
  reporting omitted materializations and performance caches as rebuildable.
- Object GC now traces artifact envelopes through deterministic directory, file,
  blob, chunk-list, and chunk edges from generation, attempt, snapshot, attestation,
  quarantine, hold, layer, and materialization roots, then reclaims last-reference
  content in restartable deterministic batches.
- Lane-space and cache-GC reports now expose artifact logical, unique authoritative,
  cross-artifact shared, materialized, lane-private, persisted-prefetch,
  demand-loaded, reclaimable, and unknown byte accounting without counting a CAS
  object more than once.
- Object GC now orders unreachable artifact DAGs parent-before-child across
  transaction batches, allowing an interrupted collection to reopen and resume
  without leaving the remaining CAS graph invalid.
- Artifact validations now distinguish structural, loadability, framework,
  policy, gate, and reproducibility declarations and produce deterministic,
  secret-rejected receipts bound to the exact desired identity and tree.
- Workspace-layer publication now rechecks exact construction pins, freezes and
  rescans Trail-owned candidate output, and requires structural and policy host-seal
  receipts before a ready artifact envelope can be published or attached.
- Artifact producers now use a host-selected phase/trust-tier capability ceiling for
  reviewed built-ins, certified signed plugins, locally trusted plugins, and repository
  declarations; signatures authenticate origin without implicitly elevating authority.
- Secret-consuming artifact producers now carry typed non-secret taint evidence;
  resolver candidates stay out of shared CAS, runtime-secret generations cannot promote
  private output, and producer receipts are rejected if tainted or sensitive while
  bounded failure evidence remains exact-value redacted.
- Ready artifact envelopes now receive deterministic content-addressed host attestations
  with typed producer, capability, policy, validation, portability, and taint evidence;
  inspection and attachment verification detect state/signature tampering and recheck
  current plugin package and publisher revocation.
- Resolver plans now fail before attempt publication when paths, arguments, or declared
  resource limits exceed host ceilings; native command-recipe tests also prove nested
  child execution remains denied.
- Repository environment parsing now recognizes an explicit `trail.environment/v2`
  header without changing v1 command semantics, and rejects mixed schema versions across
  one local include/profile graph.
- Version-2 repository documents now retain typed resolver, action-phase, validation,
  capability, heterogeneous-output, and source-export declarations with strict nested
  unknown-field rejection; v1 documents cannot opt into those fields implicitly.
- Repository v2 pipelines now compile into Trail's shared discovery, resolution,
  component-graph, desired-key v2, output, validation, and source-export models instead
  of introducing a parallel framework-specific execution representation.
- Repository v2 loading now bounds and canonicalizes argv, inputs, authorities, actions,
  validations, and exports, and rejects shells/control flow, indirect child launchers,
  absolute host paths, raw secrets, provider sockets, forbidden executable phases,
  capability escalation, compatible reuse, and host-wide reuse before tool resolution.

### Changed

- Changed omitted lane workdir mode to lazy qualified transparent `auto`.
- Replaced the old environment sync spellings with `trail env sync all` and
  `trail env sync component`.
- Added framework-neutral output policy, reuse, scope, publication, cache
  decision, and rebuild provenance.
- Added journaled `trail env promote` publication of quiesced private outputs.

### Fixed

- Concurrent materialized-lane initialization now retries short SQLite WAL
  checkpoint contention and allows native Linux observer fences enough delivery
  time under high startup fan-out.
- Windows backup publication retries permission-denied file and directory syncs
  across both handle opening and `sync_all`, preventing transient sharing-state
  failures from aborting an otherwise complete backup.
- Backup verification and restore now validate private staged SQLite copies under
  their own write lock, allowing Windows WAL/SHM initialization during handoff
  without rejecting a valid schema generation.

## [0.2.0] - 2026-08-07

### Changed

- **Breaking:** Trail's SQLite database is now schema v1. The former v18–v21
  migration chain and compatibility fixtures are removed; existing non-v1
  workspaces must be backed up and reinitialized with `trail init --force`.

### Fixed

- Terminal-agent `--workdir-mode auto` now selects a supported transparent COW
  backend for environment-backed tasks, while retaining native/portable
  fallback on hosts without one.
- Agent apply releases its temporary layered-workdir mount before checking
  merge readiness, so an automatic COW lane no longer reports its own mount as
  an active writer.
- Backup restore re-secures private `.trail` directories and permits the
  restored changed-path scope to rebind to the current host on its next daemon
  startup.
- Terminal-agent starts now return the recorded checkpoint operation for
  layered COW workdirs, matching the materialized-workdir report contract.
- Automatic update notices use the shared terminal renderer while preserving
  structured-output silence for JSON commands.
- Lane archive and unarchive daemon requests no longer send an unexpected JSON
  body, and interrupted observer retirement with a failed owner can be reopened
  and resumed instead of being reported as a corrupt schema.

## [0.1.1] - 2026-07-29

### Added

- Added `trail upgrade` for installation-aware stable upgrades through
  Homebrew or cargo-dist release installer receipts.
- Added `trail upgrade --check` and non-blocking, once-daily interactive
  update notices. Set `TRAIL_NO_UPDATE_CHECK=1` to disable automatic checks.

### Changed

- **Breaking:** Trail CLI human output now uses the unified outcome-first
  terminal renderer. The old human layouts and `--no-color` option are removed;
  use `--color never` instead.
- **Breaking:** `trail merge-lane` is removed. Use
  `trail lane merge <lane> --into <branch>` for lane-specific merges; the
  `trail merge` command remains for generic branch/ref merges.
- **Breaking:** `POST /v1/branches/{branch}/merge-lane` is removed. Use
  `POST /v1/lanes/{lane}/merge` with the target branch in the required `into`
  JSON field.
- **Breaking:** the generic merge queue is now lane-only. Use
  `trail lane merge-queue`, `/v1/lanes/merges/queue`, and
  `trail.lane_merge_queue_*`; the previous CLI, HTTP, MCP, resource, and
  `merge_queue` storage contracts are removed without aliases. Generic
  branches and refs continue through `trail merge`.
- Added `--format human|plain|json|ndjson`, `--color auto|always|never`, and
  `--pager auto|always|never`. `plain` is deterministic text; JSON and NDJSON
  are the supported contracts for automation.
- Status, diff, history, lane, agent, maintenance, and diagnostic output now
  use responsive tables, ordered checklists, explicit notices, and safe next
  actions. Human output is intentionally not stable for parsing.

## [0.1.0] - 2026-07-10

### Added

- Local-first operation history, branches, line provenance, and worktree recording.
- Isolated agent lanes with sessions, turns, patches, approvals, gates, and handoffs.
- Conflict-aware lane merges, merge queues, readiness reports, and recovery checkpoints.
- CLI, HTTP daemon, MCP stdio server, ACP relay, and Rust API integration surfaces.
- Backup, restore, filesystem checks, index rebuilding, and maintenance commands.

[Unreleased]: https://github.com/crabbuild/trail/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/crabbuild/trail/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/crabbuild/trail/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/crabbuild/trail/releases/tag/v0.1.0
