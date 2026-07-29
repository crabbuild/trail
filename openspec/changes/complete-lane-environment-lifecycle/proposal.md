## Why

Trail already separates source changes from framework-generated artifacts and can share immutable workspace layers, but lane deletion currently preserves the view and its active environment generation, lane forks do not inherit compatible environment bindings, and execution entry points do not consistently prepare or dispose environments. The missing lifecycle makes disposable artifacts accumulate, prevents automatic COW reuse, and allows agents to run outside the environment model.

## What Changes

- Add distinct lane archive, remove, and purge operations with durable lifecycle states and idempotent crash recovery.
- Make removal stop Trail-owned runtime resources, retire active environment generations, unbind immutable layers, delete generated and scratch uppers, preserve compact provenance, and make unreferenced layers eligible for cache GC.
- Make purge erase the remaining lane provenance and private storage after retirement; archive remains reversible and retains the lane's data.
- Permit a removed or purged lane name to be spawned again without colliding with historical records.
- Inherit compatible immutable environment generations and layer bindings by reference when a lane is spawned with `--from <lane>`, while creating new source, generated, and scratch uppers for the child.
- Route terminal agents, ACP sessions, tests, evals, and lane execution through one environment lifecycle: spawn, discover/plan, sync-all, reconcile, mount, execute, checkpoint durable source, and dispose runtime/private artifacts.
- Add crash-injection, recovery, compatibility, isolation, and end-to-end tests for the lifecycle.

## Capabilities

### New Capabilities

- `lane-retirement`: Archive, remove, purge, recovery, provenance retention, artifact disposal, and same-name reuse semantics.
- `lane-environment-inheritance`: Reference-based inheritance of compatible immutable environment state with fresh private uppers.
- `managed-execution-lifecycle`: A shared environment preparation and cleanup lifecycle for all Trail execution entry points.

### Modified Capabilities

None. The repository has no established main OpenSpec capabilities for these behaviors.

## Impact

The change affects lane command/API/MCP surfaces, lane and workspace-view schema/state, environment generation and layer bindings, runtime leases and mounts, cache GC roots, lane spawn, terminal-agent and ACP launch paths, test/eval runners, and lane execution. Existing `lane rm` behavior becomes explicitly disposable; users who need reversible retention use `lane archive`.
