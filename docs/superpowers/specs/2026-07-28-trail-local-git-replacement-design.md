# Trail Local Git Replacement Design

**Status:** Approved design

**Date:** 2026-07-28

## Purpose

Trail will completely replace Git for local work between published commits while
retaining Git for remotes, pull requests, commit exchange, and ecosystem
compatibility.

While Trail owns a workspace, Trail is the authority for local branches, lanes,
operations, checkpoints, rewinds, merges, provenance, review state, and accepted
local history. Direct mutating Git commands are unavailable except through an
explicit release or publication transition. Publication materializes the exact
accepted Trail delta into the normal Git worktree and stages it in the normal Git
index. The user then runs `git commit` and `git push`.

This design is production-blocking on macOS/APFS, Linux/ext4, Linux/XFS, and
Windows/NTFS at the following envelope:

- 1,000,000 Git-tracked paths.
- 128 simultaneously active lanes.
- 64 concurrent operations.
- 50,000 changed paths per lane.

Existing `.trail` workspaces must upgrade in place without losing lane history,
provenance, mappings, recovery state, or retained operations.

## Product Boundary

Trail owns local, unpublished work. Git owns published commit history and remote
interoperability.

Trail replaces these local Git use cases:

- Working-tree status and diff.
- Staging-like grouping of accepted operations.
- Local branches and worktrees through Trail branches and lanes.
- Stash through checkpoints and retained lane heads.
- Local merge, cherry-pick-like selective apply, and rebase-like lane-base
  advancement.
- Local reflog-like recovery through immutable operation history and receipts.
- Local blame through line and operation provenance.

Trail does not replace:

- Git remotes, fetch, pull, push, or remote authentication.
- Hosting-provider pull requests, reviews, or branch protection.
- Git LFS object transfer.
- The internal history of a submodule. A submodule is a separate Trail
  workspace; its gitlink is represented in the parent workspace.

## Workspace Ownership State Machine

The workspace has one durable ownership state:

```text
git_owned
   | trail acquire
   v
trail_owned
   | trail publish
   v
publishing
   | exact delta staged successfully
   v
git_owned_pending_commit
   | user commits/pushes, then trail acquire
   v
trail_owned
```

### `trail_owned`

Trail has exclusive local mutation authority.

- Git discovery metadata is parked transactionally so an ordinary `git`
  invocation from the workspace cannot discover a repository.
- Trail retains explicit read-only access to the parked Git repository for
  baseline verification and object lookup.
- Every mutation goes through the workspace coordinator or the owning lane
  actor.
- The normal Git worktree and index remain fixed at the imported baseline until
  publication.
- An out-of-band change to the worktree, parked repository, ownership marker,
  projection identity, or authenticated owner revokes authority and fails
  closed.

### `publishing`

The coordinator has exclusive publication authority. Lane mutation, accepted
target mutation, and another publication attempt are rejected until publication
completes, rolls back, or enters a repair-required state.

### `git_owned_pending_commit`

Git discovery is restored and Trail relinquishes local mutation authority.

- The normal worktree contains the accepted Trail snapshot.
- The normal Git index stages exactly the accepted delta by default.
- `trail publish --unstaged` applies the same delta but restores the original
  index.
- Trail reports the exact `git diff --cached`, `git commit`, and `git push`
  continuation commands.
- Trail does not silently retake ownership after a crash or timeout.

### `trail acquire`

Trail reacquires a clean Git workspace when either:

- `HEAD` is the imported baseline and the exact publication remains staged; or
- `HEAD` is a descendant of the imported baseline and its committed tree
  incorporates the published root.

Trail imports resulting commits and mappings, verifies the worktree and index are
clean, parks Git discovery, and returns to `trail_owned`. Dirty, rewritten,
divergent, or structurally ambiguous Git state is rejected with a stable
machine-readable reconciliation report.

## Storage Architecture

Trail separates global coordination from lane-local mutation.

```text
Workspace coordinator
|-- workspace ownership and publication state
|-- accepted branches and merge queue
|-- Git mappings and publication receipts
|-- lane registry and immutable lane-head receipts
`-- shared content-addressed object store

Lane shard x 128
|-- operations and parent relationships
|-- checkpoints, sessions, provenance, and approvals
|-- changed-path candidates and projection cursor
|-- initialization and recovery journal
`-- one serialized writer actor
```

### Coordinator database

The coordinator database stores only workspace-wide authority:

- Workspace identity, format version, and ownership state.
- Accepted Trail branches and target-root compare-and-swap state.
- Lane registry and immutable lane-head receipts.
- Merge queue, merge receipts, conflicts, and accepted resolution receipts.
- Git baselines, imported commit mappings, and publication receipts.
- Shared-object reachability roots, retention policy, and garbage-collection
  epochs.
- Projection-provider registry and authenticated mount identities.

### Lane shards

Each lane has an independent SQLite shard at
`.trail/lanes/<lane-id>/lane.sqlite`.

- Different lane shards can accept writes concurrently.
- One lane actor serializes mutations within that lane.
- A lane shard owns its operation graph, checkpoints, provenance, approvals,
  sessions, observer cursor, changed-path state, and recovery phases.
- Opening or recovering one lane does not checkpoint, rotate, or retire another
  lane's WAL or observer state.

### Cross-boundary receipts

There are no cross-database transactions. Cross-boundary actions use immutable,
content-addressed receipts.

A lane durably produces:

```text
LaneHeadReceipt {
    receipt_version,
    workspace_id,
    lane_id,
    request_id,
    base_change_id,
    base_root_id,
    result_change_id,
    result_root_id,
    changed_path_manifest_id,
    projection_fence,
    created_at,
    integrity_digest
}
```

The coordinator validates the receipt and its referenced objects, compares and
swaps the accepted target root, and records an immutable merge receipt. Replaying
the same request returns the terminal receipt. A stale target returns a typed
retryable result without recomputing against an undisclosed root.

### Shared object store and garbage collection

Immutable file content, manifests, roots, operation payloads, and merge
artifacts remain content-addressed and append-only.

Garbage collection first records a durable collection epoch, then marks from:

- Coordinator accepted roots.
- All live and retained lane roots.
- Publication, merge, repair, and migration receipts.
- Pending initialization and recovery journals.
- Configured audit-history retention roots.

Sweep can remove only objects that were unreachable at both the start and
validated end fence of the collection epoch. Interrupted collection is
idempotently resumed or abandoned without deleting newly reachable data.

## Concurrency and Authority

The workspace coordinator is a long-lived authority service. CLI, HTTP, MCP,
editor, and agent-host mutations route to it. Lane mutations route onward to the
owning lane actor.

- Sixty-four operations may progress concurrently across different lanes.
- One lane has one ordered mutation stream.
- Accepted-target compare-and-swap, publication, migration cutover, and garbage
  collection finalization are coordinator-serialized.
- Read APIs use fenced snapshots and do not acquire unrelated lane writer
  authority.
- External mutation requests require idempotency keys.
- Actor, merge, publication, and repair queues are independently bounded.
- Queue saturation returns a stable retryable error with queue depth, operation
  class, and retry guidance.
- Deadlines never revoke a live authenticated owner.
- Cancellation takes effect only at declared durable safe points.

The observer service is coordinator-owned and aggregates projection events.
Events are routed by authenticated projection identity. Observer loss revokes
only the affected lane unless workspace-level integrity is uncertain.

## Lane Filesystem Projection

Full copies or full namespace clones cannot meet the production envelope.
Production lanes use lazy filesystem projections through a common provider
contract:

```text
create(lane, baseline_root)
mount(lane, destination)
snapshot_changed_paths(lane, fence)
apply_controlled_delta(lane, delta)
flush_and_fence(lane)
unmount(lane)
recover(lane, journal)
measure_space(lane)
```

Platform providers are:

- macOS/APFS: macFUSE-backed lazy namespace with APFS clone files for hydrated
  writable content.
- Linux/ext4 or XFS: overlayfs where permitted, with fuse-overlayfs as the
  unprivileged production path.
- Windows/NTFS: ProjFS placeholders backed by Trail objects, with NTFS block
  cloning where available.
- Portable full materialization: a compatibility mode that is not certified for
  the production scale envelope.

Every lane presents the complete repository namespace. Reads hydrate immutable
content on demand. Writes enter a lane-private upper layer and durably append
changed-path intent before acknowledgement.

Provider conformance includes:

- Create, overwrite, append, truncate, and atomic replacement.
- Delete and recursive delete.
- File and directory rename.
- Regular, executable, symlink, sparse, and memory-mapped files.
- File locks and concurrent readers.
- Unicode normalization, case collisions, reserved names, and long paths.
- Directory/file conflicts and paths changing kind.
- Crash, forced unmount, provider restart, storage exhaustion, and permission
  failures.

Projection events are authoritative because the provider writes intent before
acknowledging mutation. Native filesystem watchers remain independent corruption
detectors. If intent durability, mount identity, or continuity cannot be proven,
the lane becomes `reconciliation_required`; Trail never reports it clean.

## Local History and Merge Semantics

Every acknowledged edit belongs to an operation. Every operation advances an
immutable root with explicit parents.

The local replacement surface includes:

- Status, diff, history, provenance, checkpoint, rewind, restore, and retained
  recovery.
- Create, rename, archive, compare, and remove lane.
- Three-way merge, preview, typed conflicts, resolution, and merge queue.
- Selective operation application.
- Lane-base advancement while retaining original provenance.
- Atomic review and publication groups.
- Exact import/export mappings to Git commits.

### Streaming merge

A merge:

1. Freezes the source at a fenced `LaneHeadReceipt`.
2. Resolves the recorded base against the current accepted target.
3. Streams changed-path manifests without loading a full repository snapshot.
4. Detects renames and directory/file conflicts using content and path identity.
5. Applies non-conflicting changes to a candidate target root.
6. Persists conflicts as typed objects rather than partially changing a
   worktree.
7. Compares and swaps the accepted target root.
8. Records a merge receipt and provenance edges.

Text conflicts preserve line identity and operation attribution. Stable conflict
types cover binary, symlink, executable-bit, deletion/modify, rename/rename,
rename/delete, directory/file, case-folding, and Unicode-normalization cases.
Conflict resolution is itself a recorded, reviewable, reversible operation.

Merges of 50,000 paths use bounded batches. Memory use is bounded independently
of repository path count. Replays are idempotent.

Git compatibility covers regular files, executable files, symlinks, gitlinks,
SHA-1 object IDs, and SHA-256 object IDs. Git LFS pointers are ordinary
byte-exact files to Trail; Git remains responsible for LFS transfer.

## Publication Protocol

`trail publish` has these durable phases:

```text
PREPARED
-> WORKTREE_APPLYING
-> WORKTREE_VERIFIED
-> INDEX_INSTALLED
-> RELEASE_INTENT_DURABLE
-> GIT_DISCOVERY_RESTORED
-> RELEASED
```

### Preparation

The coordinator:

1. Acquires exclusive publication authority.
2. Freezes the accepted Trail root.
3. Verifies parked Git `HEAD`, branch, index tree, attributes, ignore policy, and
   imported mapping.
4. Calculates the exact delta between the imported Git root and accepted Trail
   root.
5. Builds a replacement normal index in a temporary file using Git plumbing.

### Worktree application

Trail applies worktree changes through a durable journal:

- New and modified files are written to temporary siblings, flushed, and
  atomically renamed.
- Deletions and directory transitions have explicit journal records.
- Affected directories are fsynced where the platform provides that guarantee.
- The imported baseline remains reconstructible from Trail objects.
- Git discovery remains parked throughout application and verification.

Trail verifies every changed path and proves unchanged Git paths remain
represented. It then atomically installs the prepared normal index.

### Release

Trail persists and flushes the complete publication receipt before restoring Git
discovery metadata. Restoring discovery is the final ownership transition.

The receipt contains:

- Imported Git `HEAD`, branch, and index tree.
- Accepted Trail change and root.
- Changed-path manifest and staged index tree.
- Worktree verification digest.
- Publication request and phase identifiers.
- Git discovery identity.
- Source binary and workspace-format versions.

Trail then reports the exact next commands. It never creates the Git commit.

### Publication recovery

Before Git discovery is restored, recovery can resume publication or reconstruct
the imported baseline. After discovery is restored, recovery verifies the
receipt and remains `git_owned_pending_commit`; it never silently parks Git
again.

`trail publish abort` is permitted only before a user commit. It verifies the
receipt, restores the imported worktree and index, parks Git discovery, and
records an abort receipt.

Fault qualification covers every durable phase, concurrent attempts, branch
movement, hook-modified index state, partial writes, disk exhaustion, permission
failure, Windows file-lock interference, daemon death, and user interruption.

## Reliability and Operational Safety

Every durable mutation has:

- A versioned request fingerprint.
- An idempotency key.
- A monotonic durable phase.
- An authenticated owner identity and fencing token.
- A terminal success, refusal, rollback, or repair-required receipt.

Trail never reports a committed mutation as absent.

Workspace and lane quotas bound:

- Queue depth.
- Process and actor count.
- Resident memory.
- Hydrated and upper-layer bytes.
- Object, journal, and WAL growth.
- Open files, handles, mounts, and temporary publication artifacts.

`trail doctor --production` is read-only and reports:

- Coordinator and lane actor health.
- Queue latency and service-time percentiles.
- Projection mounts, hydration, and upper-layer size.
- Observer continuity and current fences.
- WAL and checkpoint health for each shard.
- Pending recovery and publication phases.
- Object reachability and garbage-collection debt.
- Git ownership state and the last publication receipt.

Repair commands preview mutations and write repair receipts.

Secrets, Git credentials, auth tokens, hook output, ignored private files,
environment variables, and absolute user paths are excluded from operation
payloads and evidence by default.

## In-Place Migration

Migration from the current single database is offline and resumable:

1. Acquire exclusive ownership and verify Trail and Git integrity.
2. Create a checksummed backup of `.trail`, including database, WAL, SHM,
   sidecars, and workdir metadata.
3. Create the new coordinator and lane shards beside the old database.
4. Copy global state into the coordinator and lane-local state into
   deterministic shards.
5. Recompute root, operation, provenance, mapping, and receipt digests.
6. Cross-check counts and object reachability against the old database.
7. Convert or remount materialized lanes through the selected projection
   provider.
8. Atomically switch the workspace-format marker.
9. Retain the old database read-only for the configured rollback window.

Failure before cutover leaves the old workspace authoritative. Older binaries
refuse the new format after cutover. Explicit rollback is allowed only before a
new-format mutation and only after verifying the backup digest.

## Production Gates

The full gate runs on macOS/APFS, Linux/ext4, Linux/XFS, and Windows/NTFS with:

- 1,000,000 tracked paths.
- 128 active lanes.
- 64 concurrent operations.
- 50,000 changed paths per lane.
- Text, binary, symlink, executable, rename, deletion, gitlink, Unicode, case,
  and long-path workloads.

### Performance ceilings

On a published reference-machine profile:

| Operation | p95 | p99 |
|---|---:|---:|
| Lazy lane creation | 3 seconds | 5 seconds |
| Warm lane status | 250 ms | 500 ms |
| Warm changed-path listing | 500 ms | 1 second |
| Record 1,000 changed paths | 2 seconds | 4 seconds |
| Record 50,000 changed paths | 15 seconds | 25 seconds |
| Merge 50,000 disjoint paths | 20 seconds | 30 seconds |
| Publish and stage 50,000 paths | 30 seconds | 45 seconds |
| Crash recovery per affected lane | 10 seconds | 30 seconds |

### Resource ceilings

- Coordinator plus 128 idle mounted lanes uses at most 4 GiB RSS.
- Each idle lane adds at most 16 MiB RSS.
- Metadata storage uses at most
  `1.20 x unique modified bytes + 4 KiB per changed path + 8 MiB per lane`.
- WAL, journals, hydration cache, temporary indexes, handles, processes, mounts,
  locks, and temporary files remain bounded.
- Warm work is proportional to changed paths rather than repository paths.

### Correctness gates

Qualification permits zero:

- Lost acknowledged operations.
- False-clean results.
- Missing or unintended merge or publication paths.
- Duplicate terminal receipts.
- Corrupt roots or unreachable referenced objects.
- Leaked owners, mounts, locks, temporary indexes, or publication artifacts.
- Changes to Git `HEAD`, refs, normal index, or discovery metadata before the
  release phase.

### Qualification schedule

- Every pull request: unit, property, migration, protocol, and focused fault
  tests.
- Nightly: 100,000 paths, 32 lanes, and 16 concurrent operations on all
  production platforms.
- Weekly: the full one-million-path envelope and a 24-hour mixed-operation soak.
- Release candidate: three consecutive full passes per filesystem and 10,000
  randomized crash/fault cuts.
- Security: threat-model review, dependency audit, malformed-repository corpus,
  path traversal and symlink tests, credential-redaction tests, and
  privilege-boundary review.
- Compatibility: SHA-1 and SHA-256 repositories, detached and unborn baselines,
  linked worktrees, hooks, submodule gitlinks, LFS pointers, partial clones, and
  common IDE filesystem behavior.
- Rollout: internal dogfood, opt-in canary, platform beta, then general
  availability.

No rollout stage advances with an unresolved severity-one correctness issue or a
waived blocking gate.

Every qualification run produces a checksummed evidence bundle containing the
source revision, binary digest, machine and filesystem profile, workload seed,
command receipts, latency and RSS samples, integrity results, fault
attestations, and original Git-state preservation.

## Acceptance

Trail is production-ready as the exclusive local Git replacement only when all
of the following are true:

1. The ownership state machine prevents accidental local Git mutation and
   recovers deterministically from every durable phase.
2. Independent lane shards and projection providers meet the full concurrency
   and repository-size envelope on every required filesystem.
3. Local history, merge, conflict, selective apply, base advancement, rewind,
   and provenance workflows are complete without local Git commands.
4. `trail publish` materializes and stages exactly the accepted delta without
   creating a commit or mutating Git before the release phase.
5. Existing workspaces migrate in place with verified history and a bounded
   rollback window.
6. All blocking correctness, performance, resource, compatibility, security,
   fault, and soak gates pass without waiver.
