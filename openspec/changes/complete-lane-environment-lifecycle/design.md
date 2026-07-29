## Context

Trail has two storage classes in a layered lane: checkpointed source changes and disposable dependency/generated/scratch data. Immutable `workspace_layers` and `environment_generations` already model shareable framework state, while per-view uppers isolate writes. The current `lane rm` removes the lane ref and workdir but retains the workspace view, active-generation pointer, generation rows, and layer bindings. Forking a lane creates a view without inheriting the parent's active immutable generation. Execution entry points also mount and launch independently of environment discovery, synchronization, runtime reconciliation, checkpointing, and cleanup.

The implementation must preserve existing changed-path observer retirement, lane-initialization retry identity, runtime ownership labels, and immutable layer verification. Filesystem and runtime-provider mutations cannot be part of a SQLite transaction, so externally atomic behavior requires a durable, replayable operation state machine.

## Goals / Non-Goals

**Goals:**

- Give archive, remove, and purge separate, documented, idempotent semantics.
- Make a successful removal leave no active generation, layer binding, runtime allocation, mount owner, generated upper, scratch upper, or workdir for that lane.
- Retain only compact removal provenance after removal and free the former lane name.
- Recover interrupted removals automatically and make retries converge.
- Reuse compatible immutable environment state by reference on lane forks without sharing mutable uppers or runtime allocations.
- Put all managed execution surfaces behind one preparation/execution/finalization lifecycle.

**Non-Goals:**

- Copying generated upper contents between lanes.
- Sharing mutable caches whose adapter compatibility contract does not permit sharing.
- Deleting immutable layers still referenced by another active or retained generation.
- Treating archive as a space-reclamation operation.
- Hiding an execution failure when checkpointing or disposal also fails.

## Decisions

### 1. Use a durable retirement state machine

Schema v21 adds `lane_retirements`, keyed by `retirement_id` with a unique live operation per lane. It stores the former name, operation kind (`remove` or `purge`), phase, force flag, compact provenance JSON, private-path manifest JSON, last error, and timestamps.

Removal phases are:

1. `prepared`: identity, safety checks, and confined cleanup targets are durable.
2. `runtime_stopped`: Trail-owned runtime allocations and mount/observer owners are quiesced.
3. `bindings_retired`: active generation pointers are removed, generations are retired, layer bindings are deleted, and the view is marked retiring in one transaction.
4. `private_deleted`: generated/scratch/source-private/workdir state is deleted through confined, idempotent filesystem operations.
5. `completed`: the lane ref and initialization are removed, the branch becomes `removed`, the lane name is tombstoned, the full view/environment declaration rows are compacted away, and the former name is free.
6. `repair_required`: the last durable phase and structured failure are retained; retry or open-time recovery resumes from that phase.

The state machine provides logical atomicity across SQLite, filesystem, observer, mount, and runtime-provider boundaries. An incomplete removal is never reported as successful. Alternatives considered were deleting everything in best-effort order (not crash safe) and retaining all generation/view rows as provenance (prevents GC and leaks private state).

### 2. Archive is reversible; remove is disposable; purge erases history

`lane archive` changes an active branch to `archived`, removes it from default active listings and execution eligibility, and retains its ref, initialization, view, uppers, generations, and provenance. `lane unarchive` restores it after validating the retained ref and view.

`lane rm` performs the retirement state machine and retains only the lane row, removed branch summary, lane removal event, and completed retirement summary. It frees the former name by replacing the stored lane name/ref with ID-qualified tombstone values.

`lane purge` first converges removal when necessary, then deletes the compact tombstone and lane-owned provenance rows that have no external retention obligation. Purge requires `--force` and an exact lane ID when a former name is ambiguous.

### 3. Inherit a generation snapshot, not a live view

When `--from` resolves to a lane with an active compatible generation, lane spawn creates the child view normally, with fresh source/generated/scratch uppers. In the spawn association transaction it creates a child generation whose predecessor points at the parent generation, copies immutable component/output/cache provenance, and binds the same verified layer IDs to the child view. Runtime resources, secrets, sync attempts, leases, and private storage identities are never inherited.

Compatibility requires the same source root or adapter-declared reusable key, matching adapter implementation/distribution identity, matching platform/portability scope, verified ready layers, and non-secret immutable policies. If any component is incompatible, that component is left unresolved for `sync-all`; compatible siblings are still inherited. This component-level strategy avoids an all-or-nothing fork while never silently attaching an incompatible artifact.

### 4. Centralize managed execution orchestration

A new internal execution module owns:

`prepare = discover -> plan -> sync-all -> runtime reconcile -> mount`

`finalize = checkpoint durable source -> stop/dispose runtime-private artifacts -> unmount`

It returns a `ManagedExecution` guard/report containing the view, generation, mount, environment, and phase receipts. Terminal agents, ACP, test, eval, and lane exec call this module rather than assembling partial lifecycles. Finalization always runs, including launch failure, signal exit, and panic-unwind paths where Rust permits cleanup. The primary command result and any checkpoint/disposal failures are all retained in a structured aggregate outcome.

### 5. Checkpoint source; dispose framework writes

Successful and failed managed executions checkpoint only source-classified changes according to the caller policy. Generated and scratch uppers are never checkpointed. Ephemeral execution-owned runtime allocations and private artifacts are disposed at finalization; persistent lane environments can retain declared immutable generation bindings until lane removal.

## Risks / Trade-offs

- **Schema migration touches a large authoritative schema.** → Add exact v20 fixture migration, downgrade refusal, backup/restore, and shape validation tests before switching behavior.
- **Runtime providers can be unavailable during removal.** → Skip provider discovery when no owned allocations exist; otherwise record `repair_required` and expose an idempotent retry.
- **A malicious or stale path manifest could delete outside Trail storage.** → Store normalized paths and revalidate every deletion against the workspace workdir or `.trail` roots using descriptor/confined helpers.
- **Partial inheritance could obscure why a component rebuilt.** → Emit per-component inherited/rejected reasons and include parent/child generation IDs in the spawn report and lane event.
- **Managed finalization could mask the command exit.** → Preserve the command outcome as primary and append checkpoint/disposal failures as explicit secondary failures.
- **Automatic cleanup can surprise workflows that inspect build output after execution.** → Only execution-owned disposable artifacts are removed automatically; durable lane environments remain until explicit remove/purge.

## Migration Plan

1. Add schema v21 and read-only retirement reporting with no behavior cutover.
2. Add archive/unarchive and removal state-machine tests, then switch `lane rm`.
3. Recover incomplete retirements during workspace open and before conflicting spawn/remove/purge operations.
4. Add fork inheritance behind the existing lane-spawn path and verify component-level fallback.
5. Add managed execution orchestration and migrate lane exec/test/eval, then terminal agents and ACP.
6. Require focused crash matrices and full library/E2E suites before declaring the lifecycle complete.

Rollback to a v20 binary is intentionally rejected once schema v21 is opened. Before migration, the existing schema backup mechanism remains the recovery path.

## Open Questions

None. The requested completion order and existing adapter/runtime ownership contracts determine the implementation choices above.
