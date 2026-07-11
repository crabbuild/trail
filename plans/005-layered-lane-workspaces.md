# Plan 005: Layered lane workspaces

> **Executor instructions**: Implement the task graph in dependency order. A
> task is DONE only when its focused tests and mapped acceptance evidence pass.
> Keep the full objective active until every acceptance row is proven against
> current code and runtime behavior.

## Status

- **Priority**: P0
- **Effort**: XXL
- **Risk**: HIGH
- **Depends on**: `plans/004-stream-root-materialization.md`
- **Category**: architecture, performance, agent-runtime
- **Design**: `docs/design/layered-lane-workspaces.md`
- **Started at**: commit `2c99209`, 2026-07-10

## Task graph

| Task | Scope | Depends on | Status |
| --- | --- | --- | --- |
| 005.1 | Extract backend-neutral `ViewCore`; run one filesystem conformance suite through FUSE, NFS, and Dokan adapters | - | DONE (platform mount runs in 005.7) |
| 005.2 | Add lazy Trail-root provider, ranged blob projections, split uppers, mutation journal, and upper-only checkpoints | 005.1 | DONE |
| 005.3 | Persist workspace views, automatic backend selection, mount leases, recovery, local mount/exec/checkpoint lifecycle, and space reports | 005.2 | DONE |
| 005.4 | Add immutable layer store, canonical environment keys, singleflight publish, bindings, integrity, quotas, GC, and Node adapters | 005.2, 005.3 | DONE |
| 005.5 | Add managed Cargo home, shared compiler cache, private target upper, optional immutable target seeds, and gate environment identities | 005.3, 005.4 | DONE |
| 005.6 | Add isolated Git shadows plus workspace-view readiness blockers and landing integration | 005.3 | DONE |
| 005.7 | Complete crash, security, ecosystem, cross-platform, and million-path/20-agent acceptance verification | 005.1–005.6 | IN PROGRESS (native NFS and Linux FUSE green; native Windows Dokan remains a platform-CI gate) |

## Acceptance evidence matrix

| Requirement | Owning tasks | Required evidence |
| --- | --- | --- |
| Twenty agents start from a one-million-path root without twenty checkouts or root maps | 005.2, 005.3, 005.7 | Scale test records mount latency, per-view RSS, and exclusive bytes for 20 views; no eager `load_root_files` call in mounted path |
| Identical Node inputs share one immutable dependency layer safely | 005.4, 005.7 | Concurrent integration test proves one build/publish and read-only shared layer IDs with private lane writes |
| Rust agents share downloads/compiler results without mutable incremental state | 005.5, 005.7 | Concurrent Cargo integration test proves shared managed caches and distinct target uppers |
| One lane cannot alter another lane or the real Git worktree | 005.1, 005.4–005.7 | Differential isolation tests cover install, clean, checkout, reset, build, and killed agents |
| Checkpoint examines only source-upper mutations and matches native recording | 005.2, 005.7 | Instrumented equivalence/property test proves no lower/composed scan and identical root |
| Process death cannot lose uncheckpointed source edits | 005.2, 005.3, 005.7 | Kill-at-every-checkpoint/cache-publish-phase recovery suite |
| Gates identify exact source root and environment layers | 005.5 | Report/schema assertions plus test/eval integration tests |
| Readiness blocks all specified workspace states | 005.6 | One focused readiness test per blocker code |
| FUSE, NFS, and Dokan pass one behavior suite | 005.1, 005.7 | Shared state-machine suite results on all supported CI platforms |
| Git receives only reviewed, gated, explicitly landed state | 005.6, 005.7 | End-to-end Git shadow and landing tests prove real refs/index remain isolated before land |
| Space report shows negligible exclusive bytes for unchanged lane | 005.3, 005.7 | Physical-space fixture and platform reports for successive unchanged views |
| Core lifecycle works locally without hosted service | 005.3, 005.7 | Offline create/mount/exec/checkpoint/recover end-to-end test |

### Verified evidence (2026-07-10)

- The real scale test built a 1,000,000-path root and opened 20 lazy views in
  172 ms. Every view held one indexed inode before lookup, aggregate RSS grew by
  5,242,880 bytes, and unchanged lanes consumed 0 bytes of exclusive file
  storage (`scripts/verify-layered-lane-scale.sh`).
- The shared mounted operation trace passes through native Linux FUSE and
  macOS NFS. NFS also passes record/unmount/sync/remount, local lifecycle,
  checkpoint/recovery, and Git checkout/reset/clean isolation. Windows Dokan
  uses the same trace plus foreground and daemon-owned mount/unmount lifecycle
  tests in `.github/workflows/layered-workspaces.yml`. Its library and test
  targets pass an `x86_64-pc-windows-msvc` cross-check; the native mounted run
  remains the final platform-CI gate.
- Foreground `trail lane mount` ownership and cross-process `trail lane
  unmount` now use a durable stop request and lease handoff. HTTP and MCP can
  start daemon-owned mount workers and expose matching mount reports. Native
  Linux FUSE and macOS NFS lifecycle runs both checkpoint their edits after
  graceful unmount.
- Checkpointing now holds a cross-process shared/exclusive mutation barrier
  from upper mutation through journal durability and from source scan through
  ref advancement and clean-marker publication. Repeated edits to the same
  path advance a new journal epoch after every checkpoint.
- `trail lane update --from` performs a three-way merge into an unmounted,
  clean layered lane, advances the pinned base and view generation, and is
  available with typed CLI, HTTP, MCP, and OpenAPI contracts.
- A real `npm ci --ignore-scripts` fixture proves two views with identical
  manifest, lock, tool, and platform inputs bind the same immutable layer.
- A native Linux FUSE fixture now combines a 50,000-file source root with a
  real frozen lodash/Prettier install containing 1,116 layer entries. Two
  mounted lanes started in 13 ms and shared one verified immutable layer.
  Immutable and newly installed `.bin` symlinks both execute correctly.
  Overwrite, `rm -rf node_modules`, and `npm ci` in lane A left lane B and the
  layer hash unchanged; lane B allocated 0 generated-upper bytes, and
  checkpointing lane A recorded zero source paths. The reproducible entry point is
  `scripts/verify-linux-node-layer-docker.sh`.
- A real Cargo/FUSE fixture proves `CARGO_HOME` and `SCCACHE_DIR` are shared,
  `sccache` runs with incremental compilation disabled, and an immutable
  target seed supplies compiler results to a second lane. Its writable target
  remains private, `cargo clean` cannot alter the producer lane or seed, and a
  checkpoint after the build records zero source paths.
- Native macOS loopback NFS now runs equivalent real dependency acceptance
  cases. Two Node lanes share one 1,116-entry lodash/Prettier layer; mounted
  package binaries and symlinks execute, while overwrite, `rm -rf
  node_modules`, and `npm ci` in lane A leave lane B and the immutable layer
  unchanged. A Cargo producer target can be published as an immutable seed for
  lane B; the consumer reuses dependency artifacts without private copies,
  `cargo clean` cannot alter the producer or seed, and both dependency
  checkpoints record zero source paths. These tests also verify that immutable
  backing files remain read-only while the NFS COW view exposes writable modes
  required by the macOS client.
- The million-path/twenty-view acceptance test also passes inside Linux: each
  view starts with one indexed path, all 20 views are created in 91 ms, and
  unchanged views consume 0 exclusive physical bytes.
- Native NFS exec/gate/checkpoint/recovery verifies exact source-root,
  environment-key, layer-ID, and stale-gate readiness behavior with no hosted
  service.
- External process-kill tests stop checkpoint and cache-publish helpers after
  every durable phase: source sync, ref advance, lane-head update, clean
  marker, staging sync, publish marker, atomic rename, and ready-state update.
  Reopen recovery preserves source uppers and completes or safely retries each
  transition.
- Git end-to-end evidence keeps the real HEAD, index bytes, and worktree
  unchanged for unreviewed and ungated land attempts. Only the exact reviewed
  checkpoint with required passing test/eval suites can fast-forward Git.
- The current full regression is green: 165 library tests, 2 binary tests, 157
  end-to-end tests, and the Prolly integration test.

## Shared invariants

- Lane refs and Trail roots remain authoritative; mounts never move refs.
- Shared layers are immutable and atomically published.
- Writable source and generated state is private to one lane.
- Source uppers and whiteouts are durable truth; journals are recoverable
  indexes, never the sole copy of work.
- High-frequency filesystem operations do not transact SQLite per write.
- Platform adapters translate protocol details but do not own overlay
  semantics.
- Cache GC can never remove source-upper work or an active layer.
- The real Git index and refs are never writable from an agent view.

## Validation gates

Every completed task runs its focused tests plus:

```sh
make fmt-check
cargo check -p trail
cargo test -p trail
```

Task 005.7 additionally runs platform conformance and scale commands introduced
by the implementation. The final completion audit must inspect evidence for
every acceptance row; green unit tests alone are insufficient.

## STOP conditions

Stop the affected task and preserve the goal if:

- A proposed refactor would require discarding dirty user-owned changes.
- A backend cannot implement a required filesystem semantic without a
  documented platform-specific design change.
- Cache correctness would depend on an incomplete or secret-bearing key.
- A failure path could delete uncheckpointed source work.
- A benchmark reports only logical size where physical isolation is the
  acceptance requirement.
