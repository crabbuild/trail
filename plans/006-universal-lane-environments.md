# Plan 006: Universal Lane Environments

Status: IN PROGRESS

Priority: P0

Size: XXL

Depends on: Plan 005 layered lane workspace semantic core

Design:

- [Universal Lane Environments](../docs/design/universal-lane-environments.md)
- [Environment Adapter and Specification Contract](../docs/design/environment-adapter-contract.md)
- [Layered Lane Workspaces](../docs/design/layered-lane-workspaces.md)

## Outcome

Trail can discover, plan, materialize, attach, execute, explain, verify, and retire a
complete reproducible lane environment. One typed graph composes toolchains,
dependencies, generated artifacts, private build state, disposable scratch, caches,
secrets, OCI images, and runtime services across multiple ecosystems.

The implementation shares immutable physical content while every lane receives isolated
writable behavior. A lane generation is atomic, recoverable, and referenced by agent
execution provenance.

## Implemented foundation

- Schema v5 separates logical component state from versioned adapter identity and
  backfills existing Node dependency rows without copying layer content.
- A host-owned adapter executor materializes declared inputs in private staging,
  single-flights immutable publication, preserves predecessor state on failure, and
  activates bindings through transactional metadata updates.
- Adapter commands start with a cleared environment, isolated HOME/tmp directories,
  declared cache variables, and cache keys containing implementation/tool executable
  identities; read-only status never executes adapter tools.
- Built-in `trail/node@1` behavior now runs through that executor.
- Experimental `trail/cargo-target-seed@1` performs locked, offline builds keyed by the
  complete source root, Cargo/rustc identity, host target, platform, and output policy.
- `trail/go-vendor@1` proves the host is not Node/Cargo-specific: it stages a pinned
  single-module source root, runs `go mod vendor` with isolated module/build caches, and
  reuses the immutable vendor tree across lanes.
- Planned executable identities are revalidated immediately before launch while the
  selected shim path is preserved for dispatchers such as rustup.
- `trail env status|sync`, `/v1/lanes/{lane}/environment`, and
  `trail.env_status|env_sync` expose initial CLI, HTTP/OpenAPI, and MCP parity.
- Side-effect-free `env discover` and atomic `env sync-all` support nested and polyglot
  roots. All builds finish before a batch savepoint updates mounts and one generation.
- Built-in adapters publish static registration metadata, so discovery markers are no
  longer hard-coded in the scanner. `env adapters`, HTTP/OpenAPI, MCP, and the Rust API
  expose the same side-effect-free adapter catalog.
- Experimental `trail/command@1` parses a strict repository specification with bounded
  local includes and versioned profile inheritance, canonically expands `{root}` plus
  declared exact/glob inputs, emits a normal host plan, exposes grants through `env plan`,
  and executes through macOS sandbox-exec, a Linux Landlock-plus-seccomp helper, or a
  capability-free Windows AppContainer plus one-process kill-on-close Job Object. The
  native policies deny undeclared writes, host reads, other executables, network, shell,
  scripts, secrets, and child processes. Failed Windows setup terminates the suspended
  process before returning. Missing kernel support and unsupported platforms fail closed.
- Active and retired environment generations persist source root, component keys,
  layers, mounts, and predecessor identity; agent execution receives the active
  generation ID and cache GC preserves generation-referenced layers.
- Adapter-owned bindings dynamically classify nonstandard paths such as `.venv`, so
  their writes remain lane-private and are excluded from source checkpoints.
- Layer replacement writes a durable activation intent and atomically renames the old
  private upper. Recovery restores it when SQLite did not commit or finishes cleanup
  when the replacement binding committed.
- Synchronization attempts persist process/start-token ownership. Open-time recovery
  preserves a usable predecessor as stale, fails a predecessor-less first build, and
  recognizes activation that committed before the process exited.
- Schema v6 and the normalized host plan support up to 32 named outputs per component.
  One action publishes one composite immutable layer; output subpaths receive independent
  bindings and lane-private uppers, activate atomically, persist in generation provenance,
  and retire removed uppers through crash-recoverable unbind intents.
- Schema v7 makes output storage policy explicit. `writable_private` outputs have a
  durable component/output/key binding identity, a nullable layer ID, canonical key
  provenance, and bytes only in the owning lane's generated upper. Activation can seed
  or preserve compatible state, never manufactures an empty shared layer, and uses the
  same filesystem-intent/SQLite recovery protocol as immutable attachment. CLI,
  HTTP/OpenAPI, MCP, plan, sync, generation, gate, and mounted-view reports distinguish
  private outputs from immutable-seeded ones. Recipes and v1/v2 plugins can declare either
  policy; mixed policies within one component remain rejected until heterogeneous action
  publication is implemented.
- Experimental `trail/cmake-build@1` uses that private-output contract to own a build
  tree without staging `CMakeCache.txt` under a false absolute path. Synchronization
  provisions the directory; configure/build runs inside the mounted lane. A real
  Linux/FUSE fixture configures and builds two lanes, verifies mounted cache paths,
  cleans one lane, proves the other remains intact, and confirms zero shared layers.
  Equivalent macOS/NFS and Windows/Dokan evidence is wired into native workflows.
- `trail-environment-adapter-sdk` defines a bounded framed-CBOR v1 protocol. Explicitly
  installed local packages are content-addressed, append-only activated/tombstoned,
  digest-checked before every use, and sandboxed without repository/database/network or
  child-process authority. Plugins participate in catalog, discovery, plan, sync, and
  sync-all through the normalized host plan. Readiness replans built-ins, recipes, and
  plugins from the current pinned root, while mount-shadow validation scans the source
  once for the complete plan set. Conformance covers reuse, copy-up, stale refresh,
  timeout, memory overrun, crash, oversized/malformed output, child execution,
  tampering, repair, and removal.
- Immutable layer publication writes a bounded verification seal over the manifest
  object, layer summary, and filesystem directory identity. Routine cache reuse and
  attachment validate the seal without walking the artifact tree; legacy or invalid
  seals perform one full verification and self-heal. Explicit verification and
  readiness continue to hash every entry, and GC removes the seal with its layer.
- External packages support detached Ed25519 publisher authentication. A read-only
  inspection command reports the canonical payload digest; workspace-local publisher
  keys use a content-derived ID and append-only trust/revocation records. Installation
  stores the exact authenticated executable bytes, signed distributions include their
  attestation in identity, revocation fails active packages closed, and catalog/install
  reports distinguish built-ins, unsigned local packages, and publisher-authenticated
  experimental packages.
- New immutable manifests retain the canonical host-owned component key, while legacy
  manifests remain readable. Stale refresh diffs previous and current input, tool,
  platform, architecture, portability, adapter, and strategy edges without exposing
  values. Paginated `env explain`, HTTP, and MCP reports return the complete available
  change set and explicitly identify legacy/missing provenance.
- Schema v8 persists current component dependency edges and immutable generation edge
  keys. Recipes and v1/v2 plugins declare stable component IDs; the host rejects missing,
  duplicate, self, and cyclic edges before execution, finalizes a deterministic
  topological order, and injects direct upstream component keys into downstream
  canonical identity. `sync-all` prepares that order and activates one generation,
  readiness propagates desired upstream-key changes through every descendant, and
  `env explain` names the exact `dependency:<component-id>` edge. A single sync requires
  ready predecessors, while upstream-only replacement immediately marks mounted
  descendants stale and preserves their historical generation edge keys. Tests cover a
  10,000-node non-recursive chain, full cycle diagnostics, missing edges, migration,
  rollback, and native macOS shared/private execution.
- Repository-wide `sync-all` now retires components absent from the desired graph in the
  same filesystem-intent/SQLite activation. Removed immutable mounts and private uppers
  disappear atomically, state rows retire, and removing the last component records an
  inspectable empty generation; scoped discovery does not delete out-of-scope state.
- `env graph` exposes the validated desired DAG through one Rust report shared by CLI,
  HTTP/OpenAPI, and MCP. Nodes are emitted in deterministic topological order with
  roots, adapters, canonical keys, dependencies, and output ownership; edges name the
  exact upstream key and their current `ordering_invalidation` semantics. The surface is
  read-only and publishes no layer or state. CLI, HTTP, and MCP paginate by target node
  with total counts and a stable next offset, while the Rust API retains a full-graph
  method for in-process consumers.
- Full recipe graphs load the specification twice (discovery and batch planning), cache
  resolved executable identities, and validate target, mount, and source-shadow prefixes
  with ordered indexes rather than quadratic scans. A 1,000-component recipe chain
  completes through the public graph path while asserting exactly two parses; the host
  resolver separately verifies 10,000 nodes without recursion.
- The public plugin SDK provides deterministic validating builders for plans, commands,
  outputs, discovery responses, and protocol-matched responses. The helpers reject the
  common structural authoring failures locally while preserving the exact v1 wire shape.
- External packages negotiate `trail.environment-adapter/v1` or `/v2`; missing protocol
  metadata preserves v1 canonical package bytes and signatures. V2 adds a separate
  `AdapterPlanV2` plus typed staging/mounted actions without changing v1 Rust or wire
  types. Mounted plugin actions run an authenticated executable copy in the native
  sandbox against the pinned ephemeral candidate view, read only exact declared inputs,
  write only writable-private outputs plus isolated HOME/tmp, and retain the predecessor
  after nonzero exit or undeclared source access. Catalog, CLI JSON/text, HTTP/OpenAPI,
  and MCP expose package protocols. Native macOS/NFS conformance exercises two distinct
  final paths, compatible private-state preservation, failure rollback, and declared
  versus undeclared source access; Linux/FUSE and Windows/Dokan jobs run the same fixture.
  The fixture also kills Trail during an active mounted command, requires the
  parent-death watchdog to terminate the sandbox helper, reopens the backend, preserves
  the predecessor, and removes the abandoned candidate. macOS NFS recovery now uses a
  dedicated `nfs-mount.json` backend record and force-detaches a known-dead server before
  any potentially blocking mountpoint metadata access.
- Trusted built-ins and authorized protocol-v2 plugins can declare keyed
  `mounted_initialization` actions for path-sensitive
  writable-private outputs. The host runs them at the final lane mountpoint using a
  disposable upper layout, rejects source or unrelated writes, copies only declared
  outputs into activation staging, and leaves the predecessor generation untouched on
  failure or kill. Python `.venv` creation and nested multi-component `sync-all` exercise
  this path on FUSE, NFS, and Dokan CI jobs.
- Schema v9 and the normalized host graph implement typed `build_requires`,
  `runtime_requires`, `binds_after`, and `invalidates_with` edges. Legacy recipe/plugin
  dependencies remain key-compatible `build_requires` edges. Identity edges enter the
  canonical component key and propagate staleness; runtime/order-only edges retain exact
  upstream generation keys and advance atomically without rebuilding the consumer.
  Repository recipes and protocol-v2 SDK builders author the same semantics, while
  graph, plan, generation, CLI/JSON, HTTP/OpenAPI, and MCP reports expose them. Migration
  backfills schema-v8 active and historic edges without losing upstream keys.
- Schema v10 replaces built-in adapters' implicit global tool-home paths with
  content-derived, workspace-scoped, performance-only cache namespaces. Node package
  stores, Cargo registry/Git state, sccache, and Go module/build caches declare protocol,
  access strategy, and complete compatibility dimensions. Commands hold crash-recoverable
  live leases; `host_exclusive` namespaces serialize users in deterministic order, while
  certified `tool_concurrent` caches retain native parallelism. Generation, graph, plan,
  CLI/JSON, HTTP/OpenAPI, and MCP provenance expose exact namespace IDs. Cache GC uses an
  atomic maintenance barrier, skips live users, supports dry-run, and reclaims inactive
  namespace trees without affecting component readiness or immutable artifacts.
- Protocol-v2 external adapters can declare performance-only content-store,
  compiler-cache, or locked-index namespaces for their staging action. The host adds the
  exact authenticated distribution/protocol/platform identity, injects only declared
  relative environment bindings, holds crash-recoverable namespace leases, and grants
  the cache root through the macOS/Linux/Windows native sandbox. External access is
  forced to `host_exclusive`; mounted actions receive no cache access and
  `tool_concurrent` fails closed pending independent certification. Native macOS/NFS and
  Linux/FUSE plugin conformance uses distinct component keys across two lanes, proves
  one namespace is reused, and denies a parent-directory escape while preserving the
  predecessor generation.
- Schema v13 introduces metadata-only external components and active/historical
  external-artifact provenance. Built-in `trail/oci-image@1` discovers strict
  `trail.oci.toml` declarations, keys digest-pinned OCI references plus platform, and
  activates them atomically across lanes without commands, mounts, fake directories, or
  GC ownership. Protocol-v2 SDK builders expose the same external-artifact contract and
  reject plans that mix provider-owned identities with actions, caches, or filesystem
  outputs. Graph, plan, generation, CLI/JSON, HTTP/OpenAPI, and MCP surfaces carry the
  exact reference, digest, platform, provider, and cleanup owner.

Still required for this plan: OCI tag resolution and registry verification/materialization,
runtime resource allocation and health edges, late secret resolution/injection,
heterogeneous component actions, complete CMake presets/toolchains/cache/install-prefix
support, observed native Windows
release-gate evidence, signed remote plugin catalogs, WASI packaging, independent
certification, tool-concurrent external-plugin cache certification, additional
built-ins, and the full cross-platform release gate.

## Why this follows Plan 005

Plan 005 proves the filesystem substrate and initial dependency workflows. Plan 006
generalizes those mechanisms without replacing them:

- workspace layer keys become environment component keys;
- layer manifests become typed artifact manifests;
- dependency mount records become generation bindings;
- Node, Cargo, Go, CMake, and Python dependency/build state become built-in environment
  adapters with ecosystem-specific sharing policies;
- lane safety, readiness, and operation records remain authoritative.

Plan 006 must not hide a backend semantic gap behind an adapter. Linux overlayfs/FUSE,
macOS NFS, and fallback materialization must first agree on copy-up, whiteout, bulk
replacement, rename, link, and crash behavior for every binding policy they advertise.

## Scope

In scope:

- environment graph and generation domain model;
- policy classification and sharing enforcement;
- adapter normalization and declarative repository schema;
- artifact build, validation, publication, verification, and collection;
- private state and cache namespaces;
- secret references and runtime injection;
- external immutable artifacts and private runtime services;
- CLI, Rust API, HTTP, MCP, operations, readiness, and provenance;
- built-in Node, Cargo, Go, CMake, Python, command, secret, and OCI/service adapters;
- plugin SDK and conformance kit;
- cross-platform scale, security, and crash testing.

Out of scope for the first stable release:

- provisioning arbitrary cloud infrastructure;
- promising bit-for-bit rebuilds for undeclared or non-hermetic tools;
- globally shared arbitrary writable directories;
- transparent interception of every command run outside Trail;
- removing existing `trail lane deps` compatibility commands.

## Architectural constraints

1. Trail owns fingerprinting, action execution, staging, validation, publication,
   attachment, persistence, and recovery.
2. Adapters return structured declarations and observations; they do not mutate shared
   state.
3. Every output selects an explicit policy.
4. Secret values are never persisted or hashed.
5. Routine attach does not recursively verify an unchanged artifact.
6. A generation switch is atomic and requires a quiescent lane or explicit managed
   restart policy.
7. Old generations remain inspectable and recoverable through retention policy.
8. CLI, Rust, HTTP, and MCP surfaces render common report types.
9. Environment operations do not commit, branch, merge, or push Git state.
10. Unknown capabilities or policy semantics fail closed.

## Work packages

### 006.1 Domain vocabulary and compatibility shell

Deliver:

- `EnvironmentSpec`, `EnvironmentComponent`, `EnvironmentEdge`, `EnvironmentArtifact`,
  `EnvironmentGeneration`, and `EnvironmentBinding` identifiers and records;
- storage-policy, portability, lifecycle, binding, and input-role enums;
- common structured diagnostic and stale-reason types;
- `trail env` command shell and `trail lane deps` compatibility mapping;
- feature/config schema version and migration boundary.

Acceptance:

- old layer and dependency records can be projected into an initial generation without
  copying artifact content;
- every existing dependency report has an equivalent environment report field;
- unknown enum values are rejected at mutation boundaries;
- unit tests cover identifier parsing, canonical serialization, and migration rollback.

### 006.2 Graph resolver and deterministic fingerprints

Deliver:

- configuration precedence and field provenance;
- side-effect-free adapter discovery;
- graph validation, cycle detection, and output-ownership collision detection;
- canonical fingerprint engine owned by the host;
- typed build, runtime, ordering, and invalidation edges;
- `env discover`, `env graph`, `env plan`, and `env explain` reports;
- exact staleness propagation and explanation.

Acceptance:

- shuffled maps, glob enumeration, locale, and process environment do not change keys;
- irrelevant edits do not invalidate a component;
- every invalidation identifies the input and propagation edge responsible;
- cycles and overlapping binding targets fail before build actions;
- golden tests are stable across Rust versions and supported platforms.

### 006.3 Typed artifacts, verification, and generations

Deliver:

- typed artifact manifest and lifecycle state machine;
- private staging, single-flight build leases, validation, atomic publication, and
  quarantine;
- attach, sample, and full verification tiers;
- atomic environment-generation transaction and activation journal;
- predecessor retention and rollback;
- garbage-collection roots and retained-by explanations;
- logical versus unique physical byte accounting.

Acceptance:

- concurrent identical builds publish once;
- interruption at every state boundary exposes no partial artifact;
- activation failure preserves or restores the predecessor generation;
- injected corruption is found by full verification and excluded from new generations;
- warm attach performs no full tree walk;
- active and retained generation artifacts survive garbage collection.

### 006.4 Declarative command adapter and policy engine

Deliver:

- `.trail/environment.toml` parser, includes, profiles, and canonical expansion;
- generic command component with argument vectors;
- host-owned sandboxed action executor;
- filesystem, process, shell, network, script, host-path, runtime, and secret capability
  policy;
- declared input/output containment and file-type validation;
- plan-time capability presentation and execution-time observation comparison.

Acceptance:

- a user-defined code generator produces a reusable verified artifact;
- undeclared filesystem writes, network access, shell execution, and host paths fail;
- cancellation leaves source, active generation, and shared storage intact;
- plan and provenance list the same granted capabilities;
- path traversal, malicious archives, and symlink escapes fail conformance fixtures.

### 006.5 Node and framework migration

Deliver:

- Node toolchain and package-manager built-in adapter;
- npm, pnpm, and supported package-store cache strategies;
- dependency tree immutable/shared versus seed/private policy selection;
- Next.js and Vite profiles with private build state and dev runtime declarations;
- migration from current dependency layer records;
- real application scale fixtures.

Acceptance:

- multiple lanes share dependency content while package-manager mutation and deletion stay
  isolated;
- `npm ci`, pnpm install, Next build/dev, and Vite build/dev pass on advertised backends;
- lockfile, Node ABI, package-manager version, install flags, and lifecycle-script policy
  cause precise invalidation;
- `.next`, Vite cache, dev ports, and processes are lane-private;
- warm attach avoids copying or rehashing the full dependency tree;
- bulk `node_modules` replacement provides bounded-memory progress and cancellation.

### 006.6 Cargo and Rust migration

Deliver:

- Rust toolchain component;
- Cargo registry and Git cache protocol;
- private `target` binding with optional immutable seed;
- sccache integration through the compiler-cache contract;
- build-script/proc-macro environment and network provenance;
- migration from current Cargo dependency logic.

Acceptance:

- registry content and compiler cache safely serve concurrent lanes;
- one lane's `cargo clean`, profile change, or target mutation is invisible to another;
- `Cargo.lock`, rustc/toolchain, features, profile, target, relevant `RUSTFLAGS`, and
  build-script policy invalidate correctly;
- native and cross-compilation artifacts receive conservative portability;
- real workspace tests cover large registry and target trees.

### 006.7 CMake and native build adapter

Status: FOUNDATION IMPLEMENTED. `trail/cmake-build@1` now provides discovery,
deterministic CMake executable/host identity, a layer-free `writable_private` build
binding, atomic generation activation, and real two-lane Linux/FUSE isolation. Configure
is intentionally deferred to the mounted lane to preserve correct absolute paths.
Preset/toolchain/package-manager/compiler-cache/install-prefix semantics and the full
native platform matrix remain in progress.

Deliver:

- CMake configure/build graph with compiler, linker, SDK/sysroot, toolchain-file,
  generator, preset, vcpkg, and Conan inputs;
- private configure/build tree;
- ccache protocol binding;
- optional immutable install-prefix publication with relocation validation;
- Ninja and Make generator support matrix.

Acceptance:

- absolute-path-bearing CMake caches remain host/lane-private;
- compiler, SDK, generator, preset, and toolchain changes invalidate precisely;
- concurrent lanes share ccache only through certified semantics;
- an explicitly installed relocatable artifact can be published and reused read-only;
- configure, incremental compile, clean, and test scratch remain isolated.

### 006.8 Secrets, external artifacts, and runtime resources

Deliver:

- secret provider reference abstraction and injection through environment, file, or file
  descriptor;
- structured redaction and secret canary scanner;
- OCI image digest resolver and external immutable artifact record;
- BuildKit cache policy integration;
- lane-private process/container, network, volume, socket, and port allocation;
- service health, restart, reuse, cleanup ownership, and recovery;
- externally managed service references.

Acceptance:

- no secret canary appears in database, keys, manifests, logs, transcripts, checkpoints,
  exports, or diagnostics;
- tag resolution records an image digest and platform;
- two lanes receive isolated container names, networks, volumes, and port allocations;
- Trail never deletes an externally owned resource;
- service readiness and cleanup recover after daemon or client crashes;
- build secrets never enter OCI layers or cache identities.

### 006.9 Adapter SDK and plugin isolation

Deliver:

- versioned serializable adapter protocol;
- built-in and subprocess/WASI host adapters sharing normalized types;
- capability negotiation and fail-closed version handling;
- SDK helpers for inputs, tools, outputs, validation, diagnostics, and progress;
- fixture host, golden plans, secret canaries, and filesystem conformance kit;
- adapter certification metadata.

Acceptance:

- a third-party adapter can discover and build without database or mount knowledge;
- plugin timeout, crash, oversized output, and malformed response cannot corrupt state;
- adapters cannot spawn processes or access network directly through the planning API;
- contract-major coexistence keeps old generations inspectable;
- certification tier and supported platform matrix appear in status.

### 006.10 Surface integration and readiness

Deliver:

- all environment reports in Rust API;
- CLI text/JSON, HTTP/OpenAPI, and MCP parity;
- Trail operations, progress, cancellation, retry, and transcript links;
- lane readiness environment dimension;
- `env exec` execution envelope and generation provenance;
- audit and export behavior under redaction policy.

Acceptance:

- report fixtures compare equivalent Rust, CLI JSON, HTTP, and MCP structures;
- every long action returns or links to an operation;
- readiness distinguishes artifact staleness, service health, secret availability,
  workspace safety, and Git conflict state;
- command history references the exact environment generation;
- environment sync has no implicit Git mutation.

### 006.11 Cross-platform scale, fault, and security gate

Deliver:

- shared semantic suite for Linux overlayfs, Linux FUSE, macOS NFS, and materialized-copy
  fallback;
- real Next.js/Vite, Rust workspace, CMake, and OCI fixtures;
- cold/warm, single/multi-lane, metadata-heavy, and disk-pressure benchmarks;
- crash injection at build, validation, publication, binding, service, and GC boundaries;
- threat model and external security review checklist;
- performance budgets and regression dashboard data.

Acceptance:

- every advertised backend passes copy-up, whiteout, bulk replacement, rename, link,
  metadata, concurrent-reader, and crash tests;
- four or more concurrent lanes demonstrate shared physical content and isolated private
  mutations;
- no benchmark uses only a tiny synthetic dependency tree as ecosystem evidence;
- routine status and attach remain bounded by graph/manifest work rather than artifact
  tree size;
- forced cancellation and host reboot recover to a valid generation;
- disk-pressure cleanup never removes active private state or referenced artifacts.

## Dependency graph

```text
005 semantic backend
    |
    v
006.1 domain + compatibility
    |
    v
006.2 graph + fingerprints
    |
    v
006.3 artifacts + generations
    |
    +-----------> 006.4 command + policy
    |                         |
    |             +-----------+-----------+
    |             v           v           v
    |          006.5 Node  006.6 Cargo  006.7 CMake
    |             \           |           /
    |              +----------+----------+
    |                         |
    +--------------------> 006.8 runtime/secrets
                              |
                           006.9 SDK
                              |
                          006.10 surfaces
                              |
                          006.11 release gate
```

Work packages may overlap after their required shared model is stable, but no ecosystem
adapter may bypass the central artifact, policy, or generation lifecycle to ship early.

## Verification matrix

| Scenario | Linux overlayfs | Linux FUSE | macOS NFS | Copy fallback |
| --- | --- | --- | --- | --- |
| Immutable read-only tree | Required | Required | Required | Required |
| Immutable seed/private COW | Required | Required | Required | Semantically required |
| Nested whiteout and bulk replacement | Required | Required | Required | Required |
| Node install/build/dev | Required | Required | Required | Required |
| Cargo build/clean/test | Required | Required | Required | Required |
| CMake configure/build/clean | Required | Required | Required | Required |
| Python mounted `.venv` init/failure/kill | Required | Required | Required | N/A to layered mounts |
| OCI runtime | Required when engine available | Required when engine available | Required when engine available | N/A to filesystem |
| Crash recovery | Required | Required | Required | Required |
| Secret canary | Required | Required | Required | Required |

Every result records operating system, architecture, filesystem, backend version, tool
versions, fixture revision, logical file/byte counts, unique physical bytes, cold/warm
state, and verification tier.

## Performance budgets

Budgets should be fixed after baseline measurement, but release gates enforce these
relationships from the start:

- unchanged warm plan is proportional to graph and declared input size, not artifact
  tree size;
- unchanged attach performs only attach-tier checks;
- a shared immutable artifact adds near-zero physical bytes for a second lane before
  copy-up;
- copy-up cost tracks bytes actually modified;
- identical concurrent builds execute one producer action;
- metadata-heavy replacement has visible progress, bounded memory, and cancellation;
- status never blocks behind a full integrity audit unless the user requested it.

## Rollout

1. Land dormant domain types and report formats behind an experimental feature.
2. Import existing Node/Cargo workspace layers into generated environment records.
3. Add `trail env` as an opt-in command family while retaining dependency commands.
4. Enable generic recipes for trusted repositories with denied-by-default capabilities.
5. Graduate Node and Cargo after scale/backend parity.
6. Add CMake and OCI/runtime support.
7. Open the adapter SDK at `experimental` certification.
8. Make environment readiness part of default lane readiness only after false-positive
   and latency budgets pass.
9. Deprecate dependency-only configuration fields after at least one compatibility
   release; do not remove aliases until usage telemetry and migration tooling agree.

## Data migration and rollback

- Database changes are additive until environment behavior reaches parity.
- Migration records original layer IDs and creates new IDs without rewriting manifests.
- A feature flag chooses the legacy dependency path or environment path per repository.
- Rolling back the binary leaves legacy records usable and ignores new tables safely.
- A generation created by a newer contract remains inspectable even when it cannot be
  activated by an older host.
- Artifact storage layout changes use dual-read/single-write before collection of the
  old format.

## Operational metrics

Track at minimum:

- plan/sync/attach/verify latency percentiles;
- graph node count and stale-reason distribution;
- build single-flight waiters and duplicate-build rate;
- artifact logical, unique physical, retained, and reclaimable bytes;
- private copy-up bytes and whiteout counts per lane;
- cache hit/miss/corruption/eviction counts;
- activation rollback and crash-recovery counts;
- secret-provider and service-health failure rates;
- backend-specific filesystem operation latency;
- readiness blocks by environment, lane safety, task, and Git reason.

Metrics and diagnostics must not include repository source, secret values, private URLs,
or command arguments classified as sensitive.

## Stop conditions

Stop rollout and keep the feature experimental if any of the following remains true:

- a lane can observe another lane's private mutation;
- partial build output can become shared or attached;
- a secret can persist outside its approved injection lifetime;
- an adapter can bypass host capability enforcement;
- generation recovery can strand a lane without a valid predecessor;
- backend behavior differs without capability negotiation;
- routine attach requires a recursive scan of large unchanged trees;
- external cleanup ownership is ambiguous;
- environment sync mutates Git state implicitly;
- API surfaces disagree about readiness, generation, or stale reasons.

## Definition of done

Plan 006 is complete when the architecture acceptance criteria and adapter contract
acceptance criteria pass, the full verification matrix is recorded for supported
platforms, migration and rollback are exercised on existing repositories, and the
feature is usable for daily multi-agent Node, Rust, native CMake, and OCI-backed
workflows without copying entire workdirs or sharing unsafe writable state.
