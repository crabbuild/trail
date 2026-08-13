---
name: trail-lanes
description: Coordinate concurrent coding agents with Trail lanes and shared reproducible environments. Use whenever work is assigned inside a Trail lane; multiple agents need isolated task workdirs, shared dependency or build artifacts, path claims, checkpoints, tests, handoffs, readiness checks, or serialized merges; or a user asks to split repository work safely across agents without sharing writable state.
---

# Trail Lanes

Use one lane per bounded task. Trail can reuse verified immutable environment artifacts across lanes, while every lane keeps private writable source, build, cache-upper, secret, service, and scratch state. Never make agents share a writable `target`, `node_modules`, virtual environment, build tree, or source workdir directly.

## Identify the Role First

- When `TRAIL_LANE` and `TRAIL_WORKSPACE` are set, act as a lane worker. Stay in the assigned lane; do not spawn another lane or launch another coding agent.
- When coordinating several tasks from the original workspace, act as the coordinator. Create and inspect lanes, prewarm environments, assign non-overlapping scopes, and integrate completed work.
- When neither applies, inspect with `trail --format json status` before mutating anything. Initialize Trail only when the user has chosen the baseline.

Put global flags before the command. From a lane workdir, use the original root explicitly:

```sh
trail --workspace "$TRAIL_WORKSPACE" --format json lane status "$TRAIL_LANE"
```

If `TRAIL_VIEW` is set, the current process is already inside a managed mounted lane. Run project tools normally in the current directory; do not nest `trail lane exec`, remount the lane, or synchronize its environment while it is active.

## Choose the Workflow

- For two or more agents, reusable dependencies, or a prewarmed toolchain, read [concurrent-agents.md](references/concurrent-agents.md).
- For edits, checks, recording, handoff, readiness, and merge preparation in one task lane, read [worker-lifecycle.md](references/worker-lifecycle.md).

## Preserve the Safety Model

Inspect before mutating. Preview records and merges. Treat claims as coordination boundaries even when enforcement is advisory. Resolve readiness blockers rather than bypassing them with `--force`, `--direct`, `--allow-stale`, or `--allow-ignored`.

Do not merge, run the merge queue, rewind, remove a lane, refresh dependencies from the network, or overwrite dirty work unless the user or coordinator explicitly authorized that consequence. Trail lanes are local refs under `refs/lanes/<name>`; they do not create Git branches or publish changes.

Finish with a concise handoff: lane name, changed paths, checks run, remaining blockers, and the exact safe next command.
