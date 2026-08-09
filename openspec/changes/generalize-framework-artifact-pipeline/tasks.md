## 1. Freeze the Merged Baseline and Failure Evidence

- [x] 1.1 Record a design-to-code map for the shipped output policies, component DAG, generation rows, sync attempts, layer manifests, publication journal, COW bindings, inheritance, cache GC, `trail.environment/v1`, protocol v1/v2, and public reports
- [x] 1.2 Pin serialization and CLI/API fixtures for the current `WorkspaceLayerKeyV1`, layer/generation/output reports, SDK v1/v2 frames, recipe schema, and promotion reports so later changes cannot reinterpret them
- [x] 1.3 Add failing Cargo and Node fixtures proving manifest-only projects are currently omitted and discovery must return typed incomplete proposals without executing tools
- [x] 1.4 Add a failing storage fixture proving two distinct desired keys with equal file content currently allocate duplicated authoritative layer bytes
- [x] 1.5 Add a failing reproducibility fixture proving current key-derived layer identity cannot represent two different content roots for one desired key safely
- [x] 1.6 Verify `/Volumes/Workspace` and create one checkout-specific `/Volumes/Workspace/crabbuild-target/trail-<worktree>` target; set it explicitly on every compiling Cargo invocation

## 2. Proposal and Resolution Domain Model

- [x] 2.1 Add typed component proposal/status/reason/recovery models for `ready`, `resolvable`, `blocked`, `unsupported`, and `ambiguous` with deterministic ordering and bounds
- [x] 2.2 Refactor built-in, plugin, and repository discovery to return proposals from pinned markers without requiring lock state or invoking tools, network, providers, or repository code
- [x] 2.3 Add `ArtifactResolutionPlanV1` with proposal/source pins, exact executable/argv, readable inputs, candidate output, authority set, script/environment roles, limits, and validation contract
- [x] 2.4 Add deterministic `ArtifactResolutionSnapshotV1` object encoding, content identity, provenance, predecessor, verification state, and proposal-key lookup
- [x] 2.5 Add durable resolver attempts with fenced owner identity, heartbeat, cancellation, bounded output, redacted authority evidence, failure receipts, and orphan recovery
- [x] 2.6 Implement explicit resolve-all and resolve-component library operations with deliberate refresh and no time-based dependency advancement
- [x] 2.7 Add resolver success, snapshot reuse, refresh, stale source, tool/authority mismatch, credential redaction, process death, malformed output, and size-limit tests

## 3. Desired, Content, Artifact, and Binding Identities

- [x] 3.1 Add validated ID types for `ArtifactDesiredKeyV2`, tree/file/blob/chunk objects, artifact envelopes, attestations, attempts, quarantines, and generation bindings
- [x] 3.2 Implement canonical CBOR desired-key v2 over adapter/package/protocol, resolution, source closure, upstream identities, actions/tools, output/validation/export contracts, non-secret environment, platform/ABI, and policy
- [x] 3.3 Preserve exact `WorkspaceLayerKeyV1` inspection and v1/v2 conversion while preventing legacy identities from being interpreted as v2-complete
- [x] 3.4 Extend graph diffing to return deterministic first and complete invalidating edges across resolution, tool, action, output, validation, export, trust, and sandbox dimensions
- [x] 3.5 Enforce complete-source-root/private fallback when an adapter lacks certified input closure or portability evidence
- [x] 3.6 Add canonical-order, absent-versus-empty, Unicode/path, semantic-normalizer-version, one-dimension-change, irrelevant-edit, and property tests for identity stability

## 4. Deterministic Artifact Content Store

- [x] 4.1 Define versioned deterministic CBOR codecs for directory nodes, file nodes, whole blobs, chunk lists, chunks, tree roots, artifact envelopes, and their object-edge validation
- [x] 4.2 Implement streaming normalized tree ingestion with sorted entries and bounded paths, depth, entries, bytes, links, modes, metadata, and concurrent-mutation detection
- [x] 4.3 Implement whole-blob storage through 1 MiB and `fastcdc-v1` chunking above it using 256 KiB/1 MiB/4 MiB bounds plus complete-file SHA-256
- [x] 4.4 Reject absolute/traversing/non-NFC paths, case collisions, escaping links, unsupported file types, prohibited modes/xattrs, excessive sparse content, and secret-policy failures before reachability
- [x] 4.5 Implement atomic object publication and validation that safely handles pre-existing equal objects, corrupt collisions, interruption, reopen, and concurrent publishers
- [x] 4.6 Add equal-tree/different-order, equal-files/different-keys, successor-small-change, large-file-boundary, symlink/hardlink, Unicode/case, interruption, and corruption tests
- [x] 4.7 Benchmark whole-file versus chunked ingestion and record CPU, object count, logical bytes, unique bytes, and successor reuse without weakening correctness thresholds

## 5. Retrofit Existing Layer Publication and Promotion

- [x] 5.1 Extend the fresh schema creator/validator with artifact trees/envelopes, construction attempts/waiters, resolution records, attestations, quarantines, holds, and exact generation bindings while preserving schema-v1 hard-cutover rules
- [x] 5.2 Add a CAS shadow-publication mode that seals and verifies content roots beside current layer copies without using CAS for attachment
- [x] 5.3 Compare shadow content roots against current full-tree verification and fail publication on any path, metadata, size, or digest disagreement
- [x] 5.4 Change new layer publication to make content objects/envelopes authoritative and treat `.trail/cache/layers` bytes as verified reconstructible materializations
- [x] 5.5 Insert CAS sealing into the current manual/on-sync/successful-gate publication path without changing quiescence, source/generation/gate pins, or successor activation semantics
- [x] 5.6 Extend current singleflight with durable owner/waiter phase evidence and exact dead-owner fencing while retaining bounded DAG scheduling and cancellation
- [x] 5.7 Add crash points from reservation through object publication, envelope readiness, materialization, and activation; prove prior generations and live private uppers remain intact

## 6. CAS-Backed COW, Recovery, Reachability, and Space

- [x] 6.1 Add manifest-backed lazy lookup/materialization to NFS, FUSE, and Dokan integration without changing existing lower/upper/whiteout semantics
- [x] 6.2 Add verified materialization caches keyed by tree root/backend compatibility with safe clone/reflink/copy policy and no mutable alias to authoritative content
- [x] 6.3 Extend fork compatibility to verify desired key, artifact envelope, tree integrity, adapter/package trust, portability, scope, and backend support while preserving fresh mutable identities
- [x] 6.4 Extend workspace-open recovery, doctor, and fsck for incomplete CAS attempts, corrupt/missing objects, orphan materializations, legacy versus CAS-backed layouts, and exact repair guidance
- [x] 6.5 Extend backup/restore so authoritative snapshots, envelopes, attestations, objects, bindings, and retained private state survive while omitted materializations/caches are reported as rebuildable
- [x] 6.6 Implement object-graph reachability and incremental deterministic GC across generations, attempts, leases, quarantines, backups, holds, directories, files, blobs, chunk lists, and chunks
- [x] 6.7 Extend lane space/cache reports with logical, unique authoritative, cross-artifact shared, materialized, lane-private, prefetched, demand-loaded, reclaimable, and unknown bytes without double counting
- [x] 6.8 Add last-reference, shared-chunk retention, materialization eviction/rebuild, interrupted GC, parent removal, remount, lower deletion, and 1/5/20-lane accounting tests

## 7. Validation, Nondeterminism, Trust, Secrets, and Attestations

- [x] 7.1 Add typed structural, loadability, framework, policy, gate, and reproducibility validation declarations and deterministic receipts
- [ ] 7.2 Implement required host sealing checks for producer termination, unchanged pins, path containment, safe content, limits, secret policy, complete tree identity, validations, and producer trust
- [x] 7.3 Detect differing tree roots for one `(trust_scope, desired_key)`, create durable quarantine/holds, block shared attachment, and retain bounded comparison provenance
- [x] 7.4 Implement explicit quarantine list/show/resolve operations and policy-controlled lane-private fallback without relabeling candidates or clearing evidence implicitly
- [ ] 7.5 Define and enforce phase-specific capability ceilings for reviewed built-ins, certified signed plugins, locally trusted plugins, and repository declarations
- [ ] 7.6 Add secret-taint propagation so secret-consuming producers are private, non-promotable, and non-exportable and secret bytes never enter identities, objects, logs, reports, or caches
- [ ] 7.7 Implement deterministic `ArtifactAttestationV1` creation, storage, inspection, attachment validation, optional signature fields, and package/publisher revocation checks
- [ ] 7.8 Add divergent-producer, malicious-plan, undeclared-write, denied-network, child-process, secret-leak, unsafe-output, revoked-package, unsupported-sandbox, and attestation-tamper tests

## 8. trail.environment/v2 and Source Export

- [ ] 8.1 Extend the existing repository parser with explicit `trail.environment/v2` while preserving exact v1 paths, includes/profiles, defaults, validation, and errors
- [ ] 8.2 Add v2 resolution, multiple typed action phases, validations, capability declarations, heterogeneous outputs, and source-export sections with strict unknown-field rejection
- [ ] 8.3 Compile v2 documents to the same proposal, resolution, graph, desired-key, output, and report models used by built-in and plugin adapters
- [ ] 8.4 Preserve fixed argv and bounded deterministic input expansion; reject shell interpolation, control flow, absolute host paths, forbidden child execution, raw secrets, provider sockets, and over-broad reuse
- [ ] 8.5 Implement source-export planning with artifact/subtree identity, destination, collision mode, validation/gate, source pin, and explicit authorization
- [ ] 8.6 Execute source export through confined normal source writes with ignore, guardrail, secret, collision, diff, checkpoint, and Git-handoff behavior; never through artifact mounting
- [ ] 8.7 Add v1 compatibility, parser snapshots, include/profile cycles, adversarial v2 documents, stale/conflicting export, ignored destination, secret-taint, custom framework, and source-diff E2E tests

## 9. Adapter Protocol v3 and SDK

- [ ] 9.1 Define bounded `trail.environment-adapter/v3` request/response types for proposals, resolution, inputs, typed phases, validations, capabilities, identities, exports, attestations, and quarantine evidence
- [ ] 9.2 Implement exact highest-mutual-version negotiation and deterministic v1/v2 conversions that cannot obtain v3 semantics from absent fields
- [ ] 9.3 Add SDK v3 builders with canonical collections, validation errors, finite limits, example adapters, package capability declarations, and documentation
- [ ] 9.4 Repeat complete host validation for every v3 response and reject duplicate IDs, bad normalization, invalid graph/phase combinations, oversized data, unsupported required fields, and package/protocol mismatch
- [ ] 9.5 Extend plugin inspect/install/trust/revocation reports with selected protocol, resolution/export capability, certification ceiling, and content/attestation policy
- [ ] 9.6 Add golden frames, CBOR round trips, compatibility fixtures, truncation/over-limit, fuzz/property, malicious response, signature/revocation, and Linux/macOS/Windows enforcement tests
- [ ] 9.7 Run SDK unit/doc tests and native plugin verification scripts with the checkout-specific external `CARGO_TARGET_DIR`

## 10. Built-In Migration and Framework Composition

- [ ] 10.1 Migrate a fixture-only adapter through proposal, resolution, desired key v2, CAS sealing, generation activation, COW execution, export, retirement, and collection before changing real ecosystems
- [ ] 10.2 Change Cargo discovery to report `Cargo.toml` without `Cargo.lock`, add a Trail-managed lock snapshot, and qualify real `--locked --offline` construction with conservative source-root identity
- [ ] 10.3 Change Node discovery to report `package.json` without a supported lock, add package-manager-specific frozen snapshots, and preserve dependency-seed plus content-cache isolation
- [ ] 10.4 Express Vite and Next.js fixtures as build components over Node resolution/dependencies, keeping framework caches/path-bound output private unless validation certifies reuse
- [ ] 10.5 Migrate Python resolution/download artifacts while retaining path-bound virtual environments and bytecode/tool caches as private or performance-only
- [ ] 10.6 Map Go, CMake, OCI/runtime, and existing command recipes to v2/v3 identities without replacing their shipped private/external/cache semantics
- [ ] 10.7 Add Maven/Gradle-like, Bazel/Nix-like, and unknown custom fixtures through plugins or repository v2 rather than framework names in Trail core
- [ ] 10.8 Remove adapter-specific resolution/publication shortcuts only after common-path correctness, compatibility, native isolation, and performance evidence passes

## 11. Public Operations and Conformance

- [ ] 11.1 Add shared library operations/reports for resolve, artifact inspect/verify/quarantine, source export, content reachability, and CAS-aware space while retaining existing env grammar
- [ ] 11.2 Add CLI help, deterministic plain output, human rendering, JSON/NDJSON, exit categories, ambiguity behavior, and exact recovery commands for new operations
- [ ] 11.3 Add aligned HTTP routes/OpenAPI schemas and MCP tools/resources/risk annotations backed by the same library operations and reports
- [ ] 11.4 Extend managed exec/test/eval/agent/ACP preparation with explicit missing-resolution policy, pinned identities, sealing decisions, and finalization receipts
- [ ] 11.5 Build behavior-based conformance fixtures shared by built-ins, plugins, and repository v2 for discovery through collection and emit machine-readable certification reports
- [ ] 11.6 Add real-tool gates for dependency resolution, compiled incremental output, framework/bundler composition, path-bound private state, external metadata, custom pipeline, and source export
- [ ] 11.7 Add deterministic 10k/100k/1M-entry and 1/5/20-lane experiments measuring content reuse, materialization amplification, private deltas, phase latency, object count, and skipped evidence
- [ ] 11.8 Run native CAS-backed isolation/materialization/promotion/export/recovery matrices on NFS, FUSE, and Dokan owning hosts; leave unavailable gates explicitly unverified

## 12. Documentation, Compatibility, and Completion Gates

- [ ] 12.1 Update README and first-lane workflow with source, resolution snapshot, desired key, content root, artifact envelope, materialization, private upper, promotion, and source-export semantics
- [ ] 12.2 Update architecture, data model, universal environments, adapter contract, layered workspaces, storage/indexing, guardrail/security, cache/GC, and performance documents to match implementation
- [ ] 12.3 Update `trail.environment/v2`, SDK v3, CLI, HTTP, MCP, troubleshooting, doctor/fsck/backup/restore, native prerequisites, and framework composition guidance with executable examples
- [ ] 12.4 Update `CHANGELOG.md`, configuration/completions, protocol compatibility, schema-v1 backup/reinitialization, rollback, and release-evidence policy
- [ ] 12.5 Run `cargo fmt --all -- --check`, workspace check/test, and Clippy with `--locked`, applicable features, and the checkout-specific external `CARGO_TARGET_DIR`
- [ ] 12.6 Run schema-v1, storage/rebuild/backup/restore, managed execution, environment/inheritance/retirement, CLI/terminal, HTTP/MCP, SDK, changed-path, and applicable native gates
- [ ] 12.7 Run critical construction, quarantine, publication, activation, and GC tests with both `RUST_TEST_THREADS=1` and the default scheduler
- [ ] 12.8 Inspect final diff/status for only intended files, deterministic/bounded contracts, no secret or machine path, no local build artifacts, no external-checkout mutation, and no unsupported platform/performance claim
