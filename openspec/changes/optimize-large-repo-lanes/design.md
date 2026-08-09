## Context

Trail's current lane stack already contains most of the safety primitives required for fast large-repository agents:

- transparent copy-on-write workspace views on qualified NFS, FUSE, and Dokan backends;
- separate source, generated, and scratch uppers;
- immutable `workspace_layers`, typed cache namespaces, component state, atomic environment generations, and component-level fork inheritance;
- a managed execution lifecycle that discovers, plans, synchronizes stale components, reconciles runtime resources, mounts, executes, checkpoints source, and finalizes;
- built-in Node, Cargo, Go, CMake, Python, and OCI adapters plus repository command recipes and local adapter plugins.

The remaining product gap is composition and default behavior. Ordinary lane creation can still select a virtual or non-layered result unless the caller understands materialization settings. The CLI exposes `sync` and `sync-all` as separate concepts. Command recipes implement only immutable-seeded and writable-private outputs, and there is no general operation that turns a validated private build result into a reusable immutable seed. The conservative Cargo adapter also includes the complete source root in its key, so unrelated changes can prevent target-seed reuse even though the compiler cache may still hit.

This design assumes Trail remains native and local-first, works without a daemon, treats `.trail/` as private state, and keeps Git publication explicit. It also assumes the schema-v1 hard cutover: there is one fresh schema creator, `PRAGMA user_version=1`, no migration framework, and every incompatible stored shape fails closed with backup and reinitialization guidance.

### Deliverables and review context

| Deliverable | Context and consumer | Technical output | Acceptance evidence |
| --- | --- | --- | --- |
| A. Default layered lanes | Agent hosts need a safe fast path without knowing filesystem backends. Lane and agent lifecycle owners review it. | Qualified `auto` selection, lazy view mount, parent-generation inheritance, no unsafe fallback, and managed preparation receipts. | CLI and library tests plus native NFS/FUSE/Dokan acceptance proving spawn, execute, checkpoint, restart, and independent writes. |
| B. Environment sync UX | Humans should request desired state, not choose between implementation-oriented sync variants. CLI, HTTP, MCP, and SDK owners review it. | Hard-cutover `env sync` grammar, unambiguous lane inference, convergence reports, and one shared typed domain operation. | Parser/help/exit tests and cross-interface report-shape tests for full hits, partial hits, failures, and ambiguity. |
| C. Reusable artifact layers | Large generated trees need deliberate reuse without sharing live mutable directories. Adapter, storage, security, and filesystem owners review it. | Expanded output policy, canonical identity, explicit promotion, immutable manifests, successor generations, and per-output inheritance reasons. | Promotion crash matrix, corruption and secret rejection, parent/child isolation, reopen, backup/restore, and adapter conformance tests. |
| D. Large-repository scale | A correct design must remain economical at hundreds of thousands of paths and many agents. Performance and release owners review it. | DAG scheduler, singleflight builders, lazy projection/prefetch, quota-aware GC, observability, and qualification scripts. | Reproducible 10k/100k/1M-path gates plus a real public repository, with backend, warmth, latency, bytes, hit rate, and skipped evidence recorded. |

## Goals / Non-Goals

**Goals:**

- Make the shortest Trail agent workflow select isolation and reuse correctly by default.
- Keep lane creation proportional to metadata and private deltas rather than repository or dependency size.
- Reuse verified dependency and build seeds across compatible lanes while isolating all subsequent writes.
- Express reuse for arbitrary frameworks through declared inputs, outputs, policies, validation, and identity dimensions.
- Make every reuse, rejection, rebuild, publication, and collection decision deterministic and explainable.
- Preserve crash safety, bounded resource use, secret rejection, path containment, and schema-v1 hard-cutover behavior.

**Non-Goals:**

- Sharing one writable Cargo `target`, `node_modules`, CMake tree, framework state directory, database, or runtime allocation between lanes.
- Turning Trail into a general build-system scheduler or inferring complete build graphs from arbitrary repository commands.
- Treating a cache hit as correctness evidence or retaining cache bytes as the only copy of authoritative state.
- Automatically promoting unknown ignored paths or all generated output.
- Adding a remote cache protocol in this change; the local manifest and identity contract must stabilize first.
- Migrating pre-change `.trail/` databases or preserving the removed `sync-all` CLI spelling.

## Decisions

### 1. Default to a lazy transparent view, not eager environment construction

An omitted lane workdir mode resolves to `auto`. `auto` creates a persistent layered-view record using the host's qualified transparent backend but does not mount the view, copy source, or run an adapter during spawn. The first managed execution mounts it after environment convergence. `trail agent` and every other managed execution surface use the same resolver.

Backend resolution order is platform-specific and limited to backends that passed their native acceptance gate. Capability probing produces a typed report containing backend, required service/driver, mount root, qualification status, and remediation. When an environment-bearing command has no qualified transparent backend, preparation fails before command launch. Portable copy, sparse, and virtual modes remain explicit choices for workflows that do not require mounted environment outputs.

This chooses predictable performance and correctness over a silent portable-copy fallback. A copied checkout could appear to work while duplicating hundreds of gigabytes or omitting environment mounts. Eager environment synchronization at spawn was also rejected because it makes task creation slow, performs open-world execution before it is needed, and prevents cheap lane planning.

### 2. Treat managed execution as desired-state convergence

The preparation path remains one library-owned operation:

```text
resolve lane/view
  -> discover desired graph
  -> compare desired keys with active generation
  -> inherit or attach verified hits
  -> singleflight-build unresolved dependency closures
  -> atomically activate one generation
  -> reconcile runtime-private resources
  -> mount
  -> execute
```

If desired and active component/output identities match, the sync phase is recorded as `skipped`; no package manager, compiler, plugin, or repository command runs. A child spawned from a prepared parent receives a new generation that references compatible parent layer IDs and records per-output rejection reasons. It never receives the parent's private uppers, live processes, secrets, ports, leases, or synchronization attempts.

Preparation remains lazy, but `env sync` provides explicit prewarming for CI images, developer startup, or a coordinator preparing several lanes. Background speculative warming is excluded until resource budgets and cancellation semantics have production evidence.

### 3. Replace sync variants with one command tree

The CLI hard cutover is:

```text
trail env sync all [<lane>]                         # all desired components
trail env sync component <component> [--lane <lane>] # targeted repair/debugging
```


The parser removes `sync-all`; an invocation receives normal usage failure plus the new spelling. When `<lane>` is omitted, containment of the current directory by exactly one mounted view or an authenticated managed-execution context is required. Trail does not select the most recent lane or guess from branch state.

All three forms call shared library operations. CLI JSON, HTTP, MCP, and Rust use one report containing desired generation identity, prior generation identity, component decisions, build attempt IDs, layer/storage identities, bytes where known, and ordered failure reasons. Human output summarizes that report and is not a parsing contract.

Nested `sync all` was chosen over another hyphenated verb because `sync` is the domain operation and `all`/`component` are scopes. A bare optional lane alongside optional subcommands was rejected because lane names such as `all` or `component` make the grammar parser-sensitive. Retaining `sync-all` as an alias was rejected because the project has no compatibility requirement and the alias would become permanent debt.

### 4. Separate output storage policy from framework identity

The normalized output contract gains these primary storage policies:

| Policy | Published lower | Writable upper | Inherited | Typical use |
| --- | --- | --- | --- | --- |
| `immutable_shared` | required | none | by exact/compatible key | generated SDK, relocatable toolchain |
| `immutable_seed_private` | required | fresh per lane | lower only | Cargo target seed, consumer-mutable dependency tree |
| `writable_private` | none | persistent per lane | never | active target/build tree, `.next`, local database |
| `disposable` | none | execution/lane scoped | never | test scratch, transient logs |

Caches, runtimes, secrets, external artifacts, and configuration remain separate declarations because their authority and lifecycle differ. The host can narrow an adapter's certified reuse scope but cannot widen it silently.

The repository-facing declaration adds reuse and publication fields without embedding ecosystem names:

```toml
[[component.output]]
name = "compiler-results"
source = "target"
target = "target"
policy = "immutable_seed_private"
portability = "host"
reuse = "exact"
scope = "workspace"
publish = "successful_gate"
gate = "build"
```

`reuse` is `none`, `exact`, or adapter-certified `compatible`. `scope` is initially `lane`, `workspace`, or `host`; Trail may narrow it based on portability and trust. `publish` is `never`, `manual`, `on_sync`, or `successful_gate`. Repository command recipes default to exact workspace reuse and cannot declare compatible reuse unless a trusted adapter/profile supplies the compatibility contract.

### 5. Canonical keys describe the complete correctness boundary

The artifact key is a deterministic digest over:

```text
contract version
+ adapter identity, implementation version, and distribution digest
+ declared input path/content Merkle roots
+ exact identity-bearing upstream component keys
+ argv, working directory, and output contract
+ resolved tool executable identities
+ identity-affecting non-secret environment
+ target, platform, architecture, ABI, and portability dimensions
+ validation contract, reuse mode, and sharing scope
```

Adapters that cannot prove a complete input closure include the complete Trail source-root identity. This is the safe fallback used by the current Cargo target-seed adapter. Narrow Cargo or monorepo keys graduate only after fixtures prove that workspace membership, local path dependencies, features, build scripts, proc macros, configuration, target/profile, and relevant environment are covered. Compiler caches such as sccache remain the primary reuse mechanism while seed keys are conservative.

Semantic file formats may produce normalized identities only through versioned adapter logic; the host otherwise hashes bytes. Unknown environment or build-script dependencies narrow reuse rather than being ignored.

### 6. Promote private output through a quiesced, journaled publication

Promotion is not a bind mount or directory rename from a live lane. The first implementation requires no active command owner for the view and takes the workspace write lock plus the view mutation barrier. It then:

1. validates that the selected component/output is declared and promotable;
2. records a durable publication attempt with source root, environment generation, output identity, producing execution/gate receipt, owner process token, and staging path;
3. flushes the backend and snapshots the selected output into owned staging while mutations are quiesced;
4. validates containment, normalized paths, file types, links, limits, secrets, deterministic manifest order, and adapter rules;
5. writes the content-addressed manifest and sealed immutable layer;
6. in one SQLite transaction marks the layer ready, creates a successor generation, records output and gate provenance, and advances the view's active-generation pointer;
7. releases the barrier without modifying or deleting the original private output.

Recovery fences a dead owner using its process/start token. Before the activation transaction, staged bytes are unpublished and removable by attempt identity. After commit, recovery recognizes the ready layer and successor generation as authoritative. Atomic online snapshots were considered but rejected for the first delivery because backend semantics differ; a short explicit quiesce is easier to prove.

Automatic `successful_gate` promotion calls the same operation only when the gate's source root, environment generation, command fingerprint, and output identity still match. A later source or private-output mutation makes the evidence stale.

### 7. Reuse existing generation and layer truth; add publication state explicitly

`workspace_layers`, `workspace_view_layers`, component output bindings, environment generations, generation outputs, key provenance, cache namespaces, and sync attempts remain the durable base. The fresh schema-v1 shape adds:

- output-policy fields for reuse mode, scope, publication trigger, gate, manifest identity, and validation contract;
- `workspace_layer_publications` for durable attempt phase, owner token, source/generation/output identities, staging path, result layer, error, and timestamps;
- generation-output provenance linking a promoted layer to its publication attempt and producer receipt;
- cache-use counters and decision metadata needed for deterministic reports without making counters authoritative.

Content manifests remain content-addressed objects; database rows point to them. Publication phases are `prepared`, `snapshotted`, `validated`, `published`, `activated`, `failed`, and `recovered`. Only `activated` may advance a view generation. Derived hit/byte statistics may be rebuilt or dropped without changing correctness.

No migration code is added. The schema creator, validator, backup/restore shape, doctor, fsck, and schema-v1 hard-cutover fixtures change together. An existing incompatible database fails before mutation and instructs the user to back up and run `trail init --force --from-git`.

### 8. Schedule DAG nodes in parallel with per-key singleflight

Planning remains deterministic and side-effect-free. Execution uses a bounded ready queue keyed by topological index and component ID. The default concurrency comes from the existing environment resource limit and is capped by host policy. A node becomes ready only after all identity-bearing predecessors succeed. Failure cancels nodes that depend on it; independent completed layers may remain valid cache entries, but the requested generation is not partially activated.

The existing layer builder lease becomes the singleflight authority for a canonical key. One builder owns staging; other lanes wait on a bounded condition/poll path with cancellation. On completion they verify and attach the same layer. On owner death, recovery fences the lease and never reads partial staging. Reports remain deterministically sorted regardless of completion order.

Unbounded parallel subprocesses and one builder per lane were rejected because they amplify memory, disk, network, and repository build-script risk.

### 9. Mount manifests lazily and prefetch only authenticated hot sets

Layer publication produces a sorted Merkle manifest with bounded directory pages, file content IDs, logical size, physical size when available, and a seal. Workspace backends resolve metadata from the manifest and project file content on first access. The shared blob projection cache uses staging plus atomic rename and may use reflink/clone primitives only for immutable content.

Trail may derive a hot set from bounded path-access provenance of a prior successful command with the same command fingerprint, component keys, and generation identities. Prefetch is advisory, cancellable, byte/entry bounded, and never changes readiness. Reports distinguish manifest metadata, prefetched bytes, demand-loaded bytes, and copy-up bytes.

Whole-tree eager materialization and unkeyed historical prefetch were rejected because both reintroduce repository-size startup and can disclose irrelevant private access patterns across scopes.

### 10. Cache collection uses pins, quotas, and typed authority

Cache configuration defines maximum physical bytes, minimum filesystem free bytes, retention duration, build concurrency, prefetch byte/entry caps, and optional per-kind budgets. Active/retained generations, mounted views, live publication attempts, active builders, and explicit pins protect layers. Source uppers, writable-private output, runtime volumes, and secrets are never cache-GC candidates.

Collection is a separate maintenance transaction with an exclusive cache barrier. Candidates are recoverable, unpinned entries sorted by last use and stable identity. Dry-run and execution share one typed report. Trail deletes only selected paths under its authenticated cache root and records logical/physical bytes before and after. External provider caches are reported but never deleted by Trail.

For Cargo, `CARGO_TARGET_DIR` continues to point to the lane's own mounted `target`; lanes do not share one mutable target directory. Managed `CARGO_HOME` and certified sccache namespaces may be shared. Repository/worktree qualification builds remain subject to the repository disk policy: every checkout uses its own target beneath `/Volumes/Workspace/crabbuild-target`.

### 11. Make evidence a release artifact, not a prose claim

The qualification harness emits JSON plus a human summary for 10k, 100k, and 1M-path synthetic repositories and a disposable copy of a real public repository already available beneath `/Volumes/Workspace/Github`. It covers cold/warm spawn, first/no-op sync, parent-to-child inheritance, 1/5/20-lane concurrency, independent writes, promotion, recovery at every publication phase, quota GC, checkpoint, and unmount/remount.

Every run records Trail version/commit, host, OS, filesystem, backend and prerequisite versions, cache warmth, toolchain, lane/component/path counts, logical and physical bytes, bytes copied/projected/prefetched, builder count, hit/miss/rejection reasons, spawn-to-command latency, sync latency, checkpoint latency, and all skipped gates. Thresholds are defined before measurement and are not raised merely to hide a regression.

Real-repository inputs are treated as read-only. The harness uses a disposable qualification workspace and keeps `.trail/` state and generated artifacts outside the tracked checkout. Cargo commands set a per-checkout external `CARGO_TARGET_DIR` on every invocation and stop if `/Volumes/Workspace` is unavailable.

## Risks / Trade-offs

- **Default layered mode depends on native filesystem prerequisites.** → Probe before committing lane initialization, return exact remediation, retain explicit source-only modes, and require native CI evidence before enabling a backend by default.
- **A supposedly complete adapter key can reuse stale or incompatible output.** → Default to exact source-root keys, require versioned conformance evidence for narrower closures, and include every tool/policy/platform dimension in canonical provenance.
- **Promotion can capture an inconsistent mutable tree.** → Require an idle view, flush and hold the mutation barrier through the staged snapshot, validate the complete manifest, and activate only after atomic publication.
- **Quiescing a very large output can pause the lane.** → Make promotion explicit, report snapshot duration/bytes, use filesystem clones for immutable staging where qualified, and defer online snapshotting to later backend-specific work.
- **Parallel builds can exhaust host resources or execute more untrusted repository code at once.** → Apply configurable concurrency/resource bounds, preserve sandbox/network policy, cancel dependency closures, and keep planning free of execution.
- **Lazy filesystems can trade startup savings for metadata latency.** → Page manifests, bound caches, prefetch authenticated hot sets, publish cold/warm metrics, and retain a correctness-equivalent backend state-machine suite.
- **Automatic successful-gate promotion may retain large unwanted artifacts.** → Require explicit repository policy, enforce quotas, expose dry-run size estimates, and keep `manual`/`never` as conservative defaults.
- **The CLI and schema cutovers invalidate old instructions and state.** → Update all reference surfaces and examples in the same change, reject rather than migrate old state, and provide backup/reinitialize guidance.
- **Physical-byte accounting varies by filesystem and sparse/reflink support.** → Mark unavailable values explicitly and never infer reclaimed physical bytes from logical size alone.

## Migration Plan

This project has no active-user compatibility obligation, so delivery is a coordinated hard cutover rather than a rolling migration:

1. Add report/model types and the complete fresh schema-v1 shape, validator, corruption/refusal tests, backup/restore coverage, and publication recovery state machine behind no public behavior switch.
2. Expand output planning and adapter SDK contracts, implement manual promotion and per-output inheritance, and qualify isolation on each native backend.
3. Add bounded DAG scheduling, singleflight, lazy projection/prefetch metrics, and quota/pin-aware cache collection.
4. Switch omitted lane mode to lazy layered `auto`, unify every managed execution surface, and add prerequisite failure reports.
5. Cut the CLI to `env sync [all|component]`, update Rust/JSON/HTTP/MCP/OpenAPI contracts, docs, examples, changelog, and shell completions together.
6. Run focused fault matrices, schema-v1 hard-cutover tests, the workspace baseline, native filesystem gates, and synthetic plus real-repository scale qualification before making performance claims.

Rollback is source-code rollback plus restore of a backup created before the new binary initialized the workspace. An older binary must refuse the incompatible fresh-schema shape; no downgrade or in-place transformation is supported.

## Open Questions

None for the first implementation. Remote cache transport, online non-quiescing promotion, cross-workspace/organization artifact scopes, and speculative background warming are explicitly deferred until the local identity, publication, and qualification contracts have evidence.
