## Why

The merged `optimize-large-repo-lanes` change now provides Trail's framework-neutral output policies, layered COW isolation, typed component graphs, singleflight construction, explicit promotion, fork inheritance, cache management, and `trail.environment.toml` authoring foundation. The remaining limitation is narrower but fundamental: manifest-only repositories are not represented when lock state is absent, desired keys still double as layer identities, and whole directory layers duplicate equal files across successor artifacts and independently keyed builds.

Trail should extend the shipped environment model with explicit resolution snapshots and a content-addressed artifact pipeline. Adapters continue to describe framework semantics, while the existing Trail host remains the sole authority for execution, validation, publication, generation binding, recovery, and collection.

## What Changes

- Extend side-effect-free discovery with typed `ready`, `resolvable`, `blocked`, `unsupported`, and `ambiguous` proposals so Cargo, Node, Python, Java, and custom components remain visible when lock or resolution state is missing.
- Add explicit, policy-controlled resolution actions that produce immutable Trail-managed snapshots as environment metadata; snapshots never become Git source unless a user invokes a separate source-materialization operation.
- Extend the existing `trail.environment.toml` schema instead of introducing a second repository artifact file. Add resolution, validation, multi-action, capability, and explicit source-export declarations that compile to the same host-owned component graph.
- Separate the desired component key, deterministic produced-content identity, artifact envelope/attestation identity, and lane-generation binding. Preserve the existing component graph and generation model while making those identities independently inspectable.
- Replace whole-directory duplication beneath published layers with deterministic directory/file/chunk content objects and reconstructible materializations. Existing immutable lowers and private COW uppers remain the lane isolation mechanism.
- Retrofit the existing publication and promotion state machines so they seal content objects, validate deterministic manifests, activate successor generations atomically, and recover without mutating a live private upper.
- Add nondeterminism quarantine when the same desired key produces different verified content roots; permit only an explicitly reported lane-private fallback while the key is quarantined.
- Add deterministic artifact attestations and distinct resolver, constructor, validator, and mounted-execution capability profiles. Secret-consuming producers remain private and non-promotable.
- Add adapter protocol v3 and SDK types for incomplete discovery, resolution, content identities, validation, capabilities, and attestations while negotiating v1/v2 packages explicitly.
- Migrate representative built-ins and custom pipelines through the common path, with native COW/CAS evidence for dependency trees, compiled output, framework builds, path-bound private state, and source export.
- Keep artifact transport local-first. Portable identities and attestations are required, but remote cache exchange and remote execution remain deferred.

## Capabilities

### New Capabilities

- `artifact-pipeline-contract`: Incomplete discovery, resolution snapshots, extensions to `trail.environment.toml`, normalized host orchestration, and explicit source export.
- `artifact-identity-invalidation`: Separate desired, content, artifact-envelope, and generation-binding identities; precise invalidation; nondeterminism quarantine; and explainability.
- `artifact-cow-storage`: Deterministic Merkle directory/file/chunk objects integrated with existing immutable layers, promotion, COW bindings, recovery, retention, and accounting.
- `artifact-trust-execution`: Resolver/build capability separation, secret taint, sealing, attestations, trust-scoped attachment, and future-portable identities.
- `artifact-adapter-conformance`: Protocol v3, SDK and repository-schema safety, behavior-based certification, representative real-tool fixtures, and native multi-lane qualification.

### Modified Capabilities

None. The merged `optimize-large-repo-lanes` change is still present as a completed change rather than a synchronized capability baseline. These new capabilities normatively depend on and preserve its shipped output-policy, COW-isolation, promotion, generation, singleflight, inheritance, and cache-safety contracts.

## Impact

- Environment domain: extend current discovery, planning, graph, sync, managed execution, generation binding, promotion, inheritance, and report models rather than creating parallel orchestration.
- Storage: extend fresh schema-v1 creation/validation with resolution snapshots, content manifests and objects, artifact envelopes, attempts, quarantines, attestations, and reachability. Incompatible stored shapes follow the existing backup plus `trail init --force` hard cutover; no migration framework is added.
- Layer storage: retain `workspace_layers`, `workspace_layer_publications`, generation outputs, mounts, private uppers, and native backends while replacing authoritative duplicated layer bytes with verified content objects plus reconstructible materializations.
- Repository authoring: evolve `trail.environment.toml` and its includes/profiles; do not introduce `trail.artifacts.toml`.
- Adapter surfaces: add explicitly negotiated `trail.environment-adapter/v3` wire types and SDK builders while preserving deterministic v1/v2 conversion.
- Public interfaces: extend the shared Rust reports and their CLI JSON/NDJSON, HTTP/OpenAPI, and MCP projections with resolution, content identity, quarantine, attestation, source export, verification, and CAS accounting.
- Qualification: retain the merged native isolation and scale gates, then add content-deduplication, resolution, nondeterminism, source-export, and real-framework evidence.
