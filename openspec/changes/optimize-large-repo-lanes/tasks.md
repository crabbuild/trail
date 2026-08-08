## 1. Contract and Fresh-Schema Foundation

- [x] 1.1 Add public domain/report enums for output storage policy, reuse mode, sharing scope, publication trigger, component decision, and rebuild reason; cover deterministic serde shapes and unknown-value rejection.
- [x] 1.2 Extend the single fresh schema-v1 DDL with output-policy provenance, `workspace_layer_publications`, producer receipts, and cache decision accounting; keep `PRAGMA user_version=1` and `schema_meta` coherent.
- [x] 1.3 Extend schema validation, doctor, fsck, backup, restore, and index rebuild for the new authoritative fields and publication rows.
- [x] 1.4 Add schema-v1 hard-cutover tests for fresh creation, reopen, incompatible old shape refusal, savepoint rollback, corrupt publication state, backup/restore, and `init --force --from-git` guidance.

## 2. Framework-Neutral Output Planning

- [x] 2.1 Expand the internal environment output model from `immutable_seed_private`/`writable_private` to include `immutable_shared` and `disposable` with explicit mount, upper, inheritance, retention, and cleanup behavior.
- [x] 2.2 Extend `trail.environment.toml` parsing and validation with `reuse`, `scope`, `publish`, and optional `gate`; reject unsafe policy combinations, overlaps, unknown fields, and adapter-certification widening.
- [x] 2.3 Extend `trail-environment-adapter-sdk` v2 wire types and authoring helpers for the new output contract while preserving bounded decoding and denied-by-default capabilities.
- [x] 2.4 Update built-in Node, Cargo, Go, CMake, Python, OCI, and command adapters to emit explicit normalized policies and complete policy provenance without changing mutable-path isolation.
- [x] 2.5 Add adapter and recipe contract tests for exact reuse, full-source fallback, identity-affecting inputs, multiple outputs, unsafe overrides, and deterministic canonical plans.

## 3. Canonical Artifact Identity

- [x] 3.1 Implement one canonical artifact-key encoder covering contract version, adapter provenance, declared input Merkle roots, upstream keys, argv/cwd, tool identities, identity environment, output policy, validation, platform, portability, and scope.
- [x] 3.2 Persist canonical key provenance once per component key and expose edge-level comparison used by `env explain`, sync, inheritance, and promotion reports.
- [x] 3.3 Add conformance fixtures proving unchanged inputs yield identical keys and that every declared identity dimension changes the key independently.
- [x] 3.4 Keep the Cargo adapter on complete-source-root identity until a separate certified source-closure fixture proves workspace members, path dependencies, features, build scripts, proc macros, target/profile, configuration, and environment coverage.

## 4. Durable Private-Output Promotion

- [x] 4.1 Add a library promotion operation that validates lane/view/component/output eligibility, requires an idle view, acquires the workspace write lock and mutation barrier, and records a durable `prepared` attempt.
- [x] 4.2 Implement bounded staged snapshotting with backend flush, containment-safe traversal, deterministic ordering, and qualified clone/reflink acceleration that never aliases mutable bytes.
- [x] 4.3 Validate staged output for path normalization, link containment, file types, limits, secrets, manifest integrity, and adapter/user rules before layer publication.
- [x] 4.4 Publish the content-addressed manifest and sealed layer atomically, then create and activate one successor environment generation that links publication and producer evidence.
- [x] 4.5 Recover dead-owner publication attempts at every phase, preserving the prior active generation and deleting only attempt-owned staging after authentication.
- [x] 4.6 Implement `manual`, `on_sync`, `successful_gate`, and `never` trigger enforcement; reject stale gate/source/generation/output evidence.
- [x] 4.7 Add crash/fault-injection, corruption, secret, symlink, quota, concurrent mutation, reopen, backup/restore, and successful-gate promotion tests.

## 5. Output Mounting and Fork Inheritance

- [x] 5.1 Teach the shared workspace view core and NFS/FUSE/Dokan bindings to mount `immutable_shared`, seeded-private, writable-private, and disposable outputs with equivalent precedence and whiteout semantics.
- [x] 5.2 Extend lane spawn to inherit compatible outputs component-by-component, attach immutable layer IDs by reference, and create fresh uppers for every seeded/private output.
- [x] 5.3 Record parent/child generation identity plus per-output reuse or rejection reasons in lane events and typed spawn/generation reports.
- [x] 5.4 Add backend-neutral state-machine tests and native NFS/FUSE/Dokan acceptance for parent/child concurrent writes, deletes, replacements, remount, checkpoint exclusion, and immutable digest preservation.

## 6. Parallel Synchronization and Singleflight

- [x] 6.1 Implement a bounded deterministic ready-queue scheduler for environment DAG nodes, honoring all dependency edge semantics and returning results in topological/component order.
- [x] 6.2 Upgrade layer builder leases into crash-recoverable per-key singleflight with bounded waiting, cancellation, dead-owner fencing, and verified result attachment.
- [x] 6.3 Ensure a failed dependency cancels only its dependent closure, preserves independently completed valid layers, and never partially activates the requested generation.
- [x] 6.4 Add concurrency tests for two lanes requesting one key, independent parallel nodes, dependency failure, cancellation, lease expiry, owner death, and default versus single-threaded test schedulers.

## 7. Lazy Projection, Prefetch, and Cache Lifecycle

- [x] 7.1 Add sorted paged layer manifests and lazy metadata/content providers so lane startup does not copy or hash complete immutable layers.
- [x] 7.2 Add an authenticated, entry/byte-bounded hot-set record keyed by command fingerprint and exact component/generation identities; implement cancellable advisory prefetch.
- [x] 7.3 Extend cache configuration with max bytes, minimum free bytes, retention, build concurrency, prefetch bounds, and optional per-kind budgets using stable scalar config keys.
- [x] 7.4 Extend cache pin calculation to active/retained generations, mounted views, live publications, builders, and explicit pins; exclude source/private/runtime/secret state categorically.
- [x] 7.5 Implement deterministic quota/pressure GC and shared dry-run/execution reports with logical/physical accounting and authenticated confined deletion.
- [x] 7.6 Add cold/warm lazy-access, bounded-prefetch, pinning, pressure, external-cache ownership, interrupted-GC, and physical-accounting-unavailable tests.

## 8. Default Layered Lane and Managed Execution UX

- [x] 8.1 Change omitted lane workdir mode to lazy `auto` and add one typed platform backend prerequisite/qualification report shared by lane spawn, agent setup, doctor, CLI JSON, HTTP, and MCP.
- [x] 8.2 Make `auto` select only a native-qualified transparent backend and reject environment-bearing execution before launch when unavailable; retain explicit virtual/sparse/portable modes for compatible workflows.
- [x] 8.3 Verify every managed surface uses the shared desired-state preparation path and records a skipped sync phase without executing tools when the active generation is current.
- [x] 8.4 Add CLI/library tests for metadata-fast spawn, no ecosystem command during spawn, first-exec preparation, full-hit execution, mixed parent inheritance, missing prerequisites, and explicit non-layered modes.

## 9. Environment Sync CLI and Public Interfaces

- [x] 9.1 Replace the CLI parser with `env sync all [<lane>]` and `env sync component <component> [--lane <lane>]`; remove the `sync-all` variant and add usage guidance for the new spelling.
- [x] 9.2 Implement exact mounted-view/managed-context lane inference and reject absent or ambiguous contexts without recency-based fallback.
- [x] 9.3 Unify all/all-default/component dispatch on shared library convergence operations and add deterministic human, plain, JSON, and NDJSON rendering.
- [x] 9.4 Align Rust reports, HTTP routes/OpenAPI, MCP tools, annotations, errors, and pagination with sync, promotion, cache decisions, and backend prerequisite reports.
- [x] 9.5 Add parser/help/stdout/stderr/exit tests and cross-interface golden tests for hits, partial rebuilds, targeted sync, promotion, ambiguity, legacy-command rejection, and failures.

## 10. Documentation and Release Contract

- [x] 10.1 Update README primary workflows and `docs/getting-started`, `docs/lanes`, and CLI reference pages to show default layered creation, automatic first-exec sync, explicit prewarming, promotion, and parent-to-child reuse.
- [x] 10.2 Update layered workspace, universal environment, adapter contract, storage/indexing, security/redaction, readiness, HTTP, MCP, and performance design/reference documentation to match implemented policy and evidence.
- [x] 10.3 Add executable Cargo, Node, and framework-neutral command-recipe examples that distinguish shared immutable seeds, per-lane writable output, safe caches, and prohibited live mutable sharing.
- [x] 10.4 Update `CHANGELOG.md`, shell completions, compatibility notes, and schema-v1 reinitialization guidance for both breaking cutovers.

## 11. Qualification and Completion Gates

- [x] 11.1 Add deterministic 10k/100k/1M-path generators and JSON evidence reports containing host/backend/toolchain, warmth, lane/component/path counts, logical/physical bytes, copy/project/prefetch bytes, hit reasons, builder count, and phase latencies.
- [x] 11.2 Add 1/5/20-lane experiments covering one prepared parent, child inheritance, independent source/generated writes, no-op rebuilds, promotion, checkpoint, remount, cache pressure, and interrupted publication.
- [x] 11.3 Qualify a disposable workspace derived from a real public repository beneath `/Volumes/Workspace/Github` without modifying the input checkout; keep generated Trail state outside tracked source.
- [x] 11.4 Run native NFS, FUSE, and Dokan gates on owning hosts and report unavailable/skipped evidence as unverified rather than passed.
- [x] 11.5 Run `cargo fmt --all -- --check`, workspace check/test/Clippy, schema-v1 hard-cutover, lane environment/inheritance/initialization/retirement, managed execution, CLI E2E, terminal-output, doctor/fsck/backup/restore, and applicable native gates with a unique external `CARGO_TARGET_DIR` on every invocation.
- [x] 11.6 Inspect final diffs and reports for deterministic contracts, rollback/recovery, bounds, path/secret safety, no local `target/`, no external-checkout mutation, and no unsupported performance or platform claim.
