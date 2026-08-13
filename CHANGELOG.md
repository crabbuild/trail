# Changelog

All notable changes to Trail are documented in this file. Trail follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.4.0] - 2026-08-12

### Added

- Release 0.4.0.

### Fixed

- Release metadata and version bump housekeeping.

### Added

- Environment support now includes contained Go multi-module workspaces, real frozen
  Yarn Classic and Bun handoffs, project-aware `uv sync --frozen`, and modern
  CMake/Ninja/preset/toolchain/ccache/vcpkg planning. Node native addons and lifecycle
  scripts require an exact committed deny-by-default approval with platform/toolchain
  identity and sandboxed output bounds.
- Versioned experimental Bazel, Gradle, Maven, and Nix adapter packages exercise the
  common protocol-v2 host lifecycle. Nix records pure locked `/nix/store` results and a
  digest-pinned builder as provider-owned immutable identities while Trail creates only
  lane-private profile/state; it never copies the store or executes Nix in the adapter.
- Canonical ecosystem certification evidence now binds repository/tool/distribution
  identities, A → B → C ancestry, deterministic plans, caches/private outputs,
  semantic validations, identity invalidation, and hashes of every raw report. Public
  environment plans expose adapter implementation and distribution digests consistently
  through Rust, CLI JSON, HTTP, MCP, and OpenAPI.

### Fixed

- Managed lane commands now derive fixed policy, resolved executable, cache,
  and output bindings from each active environment adapter instead of injecting
  Cargo/npm defaults globally. Go, pnpm/npm/Yarn/Bun, Python, and CMake commands
  receive isolated framework-native caches and exact tool paths, while inactive
  frameworks no longer leak variables into the command. Cargo-managed commands
  also discard inherited profile, target, wrapper, and Rust-flag overrides before
  applying the pinned lane policy, so compatible dependency fingerprints remain
  reusable across lanes.
- Managed execution now rejects environment-bearing materialized lanes before
  emitting an impossible resolution command and recommends a new
  `--workdir-mode auto` lane. Layered workspace backends remain required for
  managed dependency and build projections.
- Node dependency executables, Python virtual environments, and CMake build trees now bind directly from the
  lane's generated upper, avoiding metadata-heavy build/dependency traversal
  through macOS NFS while preserving a mounted source path and lane-private
  mutation. Python exposes the physical private environment through
  `VIRTUAL_ENV`, `PATH`, and `TRAIL_VENV_PYTHON` while `.venv` remains visible
  at its conventional lane path.
- macOS NFS lane mounts now retain attributes and negative lookups for up to 60
  seconds within a mounted execution. Same-client mutations still invalidate
  cached entries and synchronous writes remain enabled, while unchanged Go and
  Node source/dependency walks avoid repeated userspace NFS round trips.
- Lane/root diff addition and deletion totals now come from the emitted text
  diff rather than stable-line identity churn, so statistics agree with the
  unified patch while line-identity inspection remains available separately.
- Built-in Claude and Codex terminal tasks now use contained launch profiles.
  Claude disables project instructions, plugins, hooks, MCP servers, skills,
  browser integration, and agents; Codex uses strict configuration, an explicit
  lane root, workspace-write sandboxing, and an empty MCP map. Both receive an
  isolated runtime home, an allowlisted environment, a lane Git shadow, and a
  typed containment receipt. On macOS, `sandbox-exec` enforces declared writable
  roots and protects the original checkout. The new
  `--allow-project-integrations` flag restores the previous project-integrated
  launch explicitly; custom commands after `--` remain unchanged. Contained
  Claude launches now preserve its documented OAuth token, OAuth-token file
  descriptor, API-key, and workload-identity variables without copying or
  discovering host keychain secrets. Claude's internal temporary directory is
  redirected into Trail's private launch runtime, and `acceptEdits` permits
  noninteractive built-in edits without disabling Bash or other higher-risk
  permission checks. An exact allowlist imports Claude credential and provider
  endpoint variables from user-level settings without importing hooks,
  plugins, permissions, or other project/global integrations; explicit process
  variables take precedence.
- Python `uv.lock` environments now select a checked `.python-version`
  major/minor interpreter, install the frozen dependency set into a copy-based
  lane-private venv, and report bounded redacted initializer diagnostics.
  Hash-pinned `requirements.lock` and verified Trail-managed lock snapshots use
  hash-required `uv pip sync`; unfrozen requirements and unsupported lock
  formats fail with recovery guidance instead of producing an empty venv.
- Transparent-COW checkpoints now apply `.trailignore` to newly created journal
  candidates before source recording, and classify `dist-node` as generated
  output. Ignored or conventional build artifacts no longer enter source merely
  because they were first observed by the native change journal.
- pnpm workspace roots now fail explicitly instead of attempting an incomplete
  `--ignore-workspace` install. Independent non-workspace pnpm projects retain
  frozen dependency-layer reuse.
- Managed execution now scopes environment discovery and synchronization to
  the command's component root, projects a verified manifest-only Cargo lock
  snapshot only for the command, and removes it before checkpointing. Unrelated
  nested projects no longer block root commands or leak generated lockfiles
  into lane source.
- Managed Cargo commands now use the active generation's declared Cargo and
  compiler cache namespaces. Lane-private mutable targets are seeded with
  owner-writable clones in the generated upper outside loopback NFS, avoiding
  metadata-heavy build I/O through the mount while preserving isolated target
  state and immutable-layer reuse.
- Cargo lock resolution now receives the declared offline `CARGO_HOME` cache
  instead of an empty isolated home, so manifest-only repositories can reuse
  the host's existing registry/Git index without network access. Cargo
  workspace members are collapsed into their owning root component, avoiding
  member-local lockfile failures, while independently nested workspaces remain
  separate components.
- Source-only Cargo lane handoffs now initialize a new source-root-exact target
  seed from the newest compatible active predecessor. Trail prefers native
  clone/reflink, Cargo revalidates the seed and recompiles affected workspace
  code, and lockfile, manifest, toolchain, target, platform, or build-policy
  changes still force an unseeded construction.
- Built-in Node dependency layers produced by a lockfile-frozen,
  script-disabled install now allow public private-key example literals in
  ordinary documentation, source, and type declarations. Strict scanning still
  rejects secret-bearing paths such as `.env`, credential, `.pem`, and `.key`
  files; custom or script-enabled producers receive no exemption.
- `trail env ... --path .` now selects the repository-root component instead
  of failing path normalization.
- Changed-path daemon authority now follows workspace generation changes and
  hands off atomically between automatic and explicit daemons. Sparse lane
  hydration also narrows native directory events to authenticated selections
  and visible files instead of treating intentionally absent siblings as
  deletions.

### Added

- Added an opt-in macOS real-framework qualification matrix for pinned bbolt,
  Polymarket CLOB TypeScript client, uuid, tappy, and LevelDB revisions. Each row
  makes deterministic production-and-test source edits and produces checksummed
  Agent A → B → C evidence for semantic ancestry, parent-generation
  inheritance, real framework tests/builds, stable dependency identity,
  cache/layer reuse, fresh lane-private outputs, incremental CMake recompilation,
  stale-output rejection, and exact checkpoint paths.
- Added deterministic 10k/100k/1M artifact and source scale matrices for
  1/5/20 lanes, fail-closed JSON evidence validation, and compositional
  owning-host NFS/FUSE/Dokan qualification that distinguishes mounted backend
  behavior from shared CAS lifecycle checks and leaves unavailable platforms
  explicitly unverified.
- MCP resource completion now returns bounded artifact-envelope and quarantine
  identities for the artifact resource templates.

- Added one behavior-based artifact conformance fixture for reviewed built-ins, local
  protocol-v3 plugins, and repository-v2 producers, plus a deterministic evidence-only
  JSON certification report covering discovery through last-reference collection.
- Added one blocking Linux real-tool artifact gate covering Cargo and npm resolution,
  compiled seed reuse, framework composition, Python/CMake private state, external
  metadata, custom repository pipelines, and guarded source export.
- Managed exec, test, eval, terminal-agent, and materialized ACP execution now fail
  before launch when an environment requires explicit resolution, pin exact source,
  snapshot, generation, and artifact-binding identities, and return additive
  preparation/finalization receipts with deterministic sealing and cleanup decisions.
- Added HTTP/OpenAPI and MCP tool/resource parity for environment resolution,
  artifact inspection/verification/reachability/accounting, quarantine
  list/show/resolve, and explicit generated-source export. All transports use
  the shared Rust reports, and MCP operations declare read-only, destructive,
  or open-world risk consistently.
- Added `trail env resolve all|component`, artifact inspect/verify/quarantine,
  and explicit source-export CLI workflows with exact discovery recovery argv,
  deterministic human/plain rendering, JSON/NDJSON reports, and stable failure
  exits. Reviewed built-in resolvers execute exact offline argv in isolated
  staging and retain fenced failure evidence; restricted repository/plugin
  resolver launch still fails closed pending its native sandbox integration.
- Added shared Rust artifact operations and serializable reports for envelope inspection,
  attach/sample/full/reproducibility-evidence verification, generation bindings,
  quarantine list/show/resolve, bounded content reachability, and CAS-aware workspace and
  per-envelope storage accounting. Existing resolution and source-export operations use
  the same public model family.
- Protocol-v2 plugins can now declare framework-neutral `verified_external` store
  identities by provider, safe reference, SHA-256 digest, and platform without creating
  layers or cleanup ownership; repository-v2 conformance fixtures cover JVM-like
  dependency/private-state and unknown custom pipelines without new core framework modes.

- Normalized environment plans now carry a host-owned protocol-v3 identity-contract
  digest alongside the exact legacy workspace-layer key. Go cache contracts exclude
  machine-local cache paths, CMake remains layer-free and lane-private, OCI/runtime
  declarations remain external metadata, and repository v2 keeps its independent
  desired-key v2 identity.
- Python environments can now bind an optional uv-generated, hash-bearing requirements
  snapshot, warm a performance-only wheel/download cache, install that snapshot with
  hash enforcement, and still keep `.venv`, its bytecode, and embedded path state
  entirely lane-private.
- `trail.environment/v2` framework fixtures now compose Next.js and Vite build/state
  components over the Node dependency component: `.next` and `.vite` remain lane-private,
  while validated Vite `dist` content can use an independently keyed immutable layer.
- Manifest-only Node components can now resolve manager-specific npm, pnpm, Yarn, or
  Bun lock snapshots into Trail metadata, then reuse the existing frozen-install
  dependency seed and performance-only content cache with lane-private COW writes.
- Cargo components without a source-tracked `Cargo.lock` are now reported as resolvable;
  a verified Trail-managed lock snapshot can be projected into isolated staging for
  real `cargo build --locked --offline` target-seed construction without entering source.
- Artifact-v2 outputs can now replace a workspace layer's compatibility CAS shadow
  with desired-key authority, activate through ordinary lane generations, remain
  isolated through copy-on-write execution, export their immutable source, and release
  generation bindings during retirement and collection.
- Added Rust library artifact resolution component/batch operations with durable fenced
  attempts, content-addressed snapshot reuse, explicit-only refresh, bounded redacted
  evidence, and deterministic reports.
- Workspace-layer singleflight now records durable generation-fenced owner phases and
  waiter outcomes, and only recovers a lock when the exact PID/start identity is proven
  dead or mismatched.
- Workspace open now recovers dead artifact constructors and exact owned staging;
  doctor and fsck validate raw CAS objects, snapshots, envelopes, attempt coherence,
  legacy/CAS layouts, and orphan materializations with repair guidance.
- Backup/restore validation now treats omitted materialization caches as disposable,
  rebases restored layer paths before publication, and parallel environment builders
  use a bounded SQLite wait during short WAL publication overlap.
- Environment discovery now reports marker-recognized plugins that do not support the
  current host as typed `unsupported` proposals without launching plugin code.
- Native lane views now resolve verified immutable artifact manifests lazily, read only
  requested blob/chunk ranges, and materialize only touched files during copy-up while
  preserving shared FUSE, NFS, and Dokan upper/whiteout semantics.
- Real-directory artifact consumers now reuse tree-root/backend-keyed verified
  materialization caches that rebuild from authoritative CAS, restore immutable
  permissions on reuse, and clone/reflink or independently copy into mutable state.
- Lane forks now inherit only individually verified CAS-backed outputs after desired-key,
  envelope/tree, current adapter package, scope, portability, and backend checks, while
  allocating fresh artifact bindings and private workspace identities.
- Portable backups now retain source uppers and authoritative artifact snapshots,
  objects, envelopes, attestations, historical generations, and exact bindings while
  reporting omitted materializations and performance caches as rebuildable.
- Object GC now traces artifact envelopes through deterministic directory, file,
  blob, chunk-list, and chunk edges from generation, attempt, snapshot, attestation,
  quarantine, hold, layer, and materialization roots, then reclaims last-reference
  content in restartable deterministic batches.
- Lane-space and cache-GC reports now expose artifact logical, unique authoritative,
  cross-artifact shared, materialized, lane-private, persisted-prefetch,
  demand-loaded, reclaimable, and unknown byte accounting without counting a CAS
  object more than once.
- Object GC now orders unreachable artifact DAGs parent-before-child across
  transaction batches, allowing an interrupted collection to reopen and resume
  without leaving the remaining CAS graph invalid.
- Artifact validations now distinguish structural, loadability, framework,
  policy, gate, and reproducibility declarations and produce deterministic,
  secret-rejected receipts bound to the exact desired identity and tree.
- Workspace-layer publication now rechecks exact construction pins, freezes and
  rescans Trail-owned candidate output, and requires structural and policy host-seal
  receipts before a ready artifact envelope can be published or attached.
- Artifact producers now use a host-selected phase/trust-tier capability ceiling for
  reviewed built-ins, certified signed plugins, locally trusted plugins, and repository
  declarations; signatures authenticate origin without implicitly elevating authority.
- Secret-consuming artifact producers now carry typed non-secret taint evidence;
  resolver candidates stay out of shared CAS, runtime-secret generations cannot promote
  private output, and producer receipts are rejected if tainted or sensitive while
  bounded failure evidence remains exact-value redacted.
- Ready artifact envelopes now receive deterministic content-addressed host attestations
  with typed producer, capability, policy, validation, portability, and taint evidence;
  inspection and attachment verification detect state/signature tampering and recheck
  current plugin package and publisher revocation.
- Resolver plans now fail before attempt publication when paths, arguments, or declared
  resource limits exceed host ceilings; native command-recipe tests also prove nested
  child execution remains denied.
- Repository environment parsing now recognizes an explicit `trail.environment/v2`
  header without changing v1 command semantics, and rejects mixed schema versions across
  one local include/profile graph.
- Version-2 repository documents now retain typed resolver, action-phase, validation,
  capability, heterogeneous-output, and source-export declarations with strict nested
  unknown-field rejection; v1 documents cannot opt into those fields implicitly.
- Repository v2 pipelines now compile into Trail's shared discovery, resolution,
  component-graph, desired-key v2, output, validation, and source-export models instead
  of introducing a parallel framework-specific execution representation.
- Repository v2 loading now bounds and canonicalizes argv, inputs, authorities, actions,
  validations, and exports, and rejects shells/control flow, indirect child launchers,
  absolute host paths, raw secrets, provider sockets, forbidden executable phases,
  capability escalation, compatible reuse, and host-wide reuse before tool resolution.
- Source-export planning now pins the lane/source and active generation, desired and
  artifact identities, exact file-or-directory subtree, destination content state,
  collision policy, validation and gate receipts, and explicit authorization without
  writing source or materializing the artifact.
- Source-export execution now revalidates every plan pin, reads bounded regular files
  directly from CAS, and applies fail/replace semantics through one normal guarded lane
  patch so ignore, secret, path, collision, diff, checkpoint, and Git-handoff behavior
  stays identical to ordinary source changes; artifact mounting is never a write path.
- Repository-pipeline compatibility coverage now snapshots the v2 source-export wire
  contract while exercising v1 planning, include/profile cycles, unsafe and
  secret-capable declarations, stale and conflicting destinations, ignored paths, a
  custom command framework, and visibility of exported source in the normal lane diff.
- The environment-adapter SDK now defines separate bounded protocol-v3 request,
  response, proposal, resolution, typed-phase, validation, capability, identity,
  source-export, attestation-requirement, secret-taint, and quarantine-evidence types
  without changing v1/v2 wire layouts or granting adapters host mutation authority.
- Adapter protocol negotiation now selects the highest exact mutual identity, and
  canonical v1/v2 projections keep every v3-only authority absent. The SDK adds a v3
  pipeline builder, explicit deny-by-default package capability declarations, detailed
  validation errors, and an artifact-pipeline example adapter.
- Trail now repeats canonical protocol-v3 validation at the plugin trust boundary,
  rejecting oversized or unknown data, duplicate IDs, non-normalized paths, input and
  host pin drift, invalid graph/phase combinations, secret-taint underclaims, and any
  package/protocol/capability/certification mismatch before normalization.
- Plugin inspection, installation, catalog/trust, and removal reports now expose one
  shared protocol-capability record with selected protocol, resolution/export and host
  evidence flags, certification ceiling, content policy, and host-attestation policy;
  the HTTP/OpenAPI adapter catalog projects the same typed fields.
- Protocol-v3 compatibility now has checked-in length-prefixed CBOR request/response
  frames, fail-closed truncation and size-limit tests, property-based exact-negotiation
  coverage, hostile host-validation cases, and an SDK contract gate on Linux, macOS,
  and Windows.

### Changed

- Cargo, Node, and Python managed-resolution consumers now use one host-owned snapshot
  verifier for proposal/source/component/adapter/format, verification state, secret taint,
  and content loading. Production adapters continue to publish only through the common
  CAS sealing and atomic generation-activation path; existing environment synchronization
  entry points remain compatibility wrappers.
- Changed omitted lane workdir mode to lazy qualified transparent `auto`.
- Replaced the old environment sync spellings with `trail env sync all` and
  `trail env sync component`.
- Added framework-neutral output policy, reuse, scope, publication, cache
  decision, and rebuild provenance.
- Added journaled `trail env promote` publication of quiesced private outputs.

### Fixed

- Managed Cargo target seeds on Linux with sccache 0.17 or newer now isolate
  each cache behind a stable abstract Unix-domain endpoint and compile in the
  attempt-owned client, preventing a persistent cache server from retaining a
  deleted staging directory as its temporary root. Older sccache versions and
  other platforms continue with Cargo incremental compilation instead of
  sharing an unsafe process-global compiler daemon.
- Initial changed-path reconciliation now retries bounded, typed SQLite busy/locked
  contention, so high-fan-out lane creation reaches observer readiness instead of
  escalating a transient WAL writer collision to committed repair.
- Linux observer fences now live under the authenticated private
  `.trail/observer-fences/` directory and ignore foreign fence nonces, preventing
  overlapping observer recovery from reporting internal sentinels as untracked source.
- Concurrent materialized-lane initialization now retries short SQLite WAL
  checkpoint contention and allows native Linux observer fences enough delivery
  time under high startup fan-out.
- Windows backup publication retries permission-denied file and directory syncs
  across both handle opening and `sync_all`, preventing transient sharing-state
  failures from aborting an otherwise complete backup.
- Backup verification and restore now validate private staged SQLite copies under
  their own write lock, allowing Windows WAL/SHM initialization during handoff
  without rejecting a valid schema generation.

## [0.2.0] - 2026-08-07

### Changed

- **Breaking:** Trail's SQLite database is now schema v1. The former v18–v21
  migration chain and compatibility fixtures are removed; existing non-v1
  workspaces must be backed up and reinitialized with `trail init --force`.

### Fixed

- Terminal-agent `--workdir-mode auto` now selects a supported transparent COW
  backend for environment-backed tasks, while retaining native/portable
  fallback on hosts without one.
- Agent apply releases its temporary layered-workdir mount before checking
  merge readiness, so an automatic COW lane no longer reports its own mount as
  an active writer.
- Backup restore re-secures private `.trail` directories and permits the
  restored changed-path scope to rebind to the current host on its next daemon
  startup.
- Terminal-agent starts now return the recorded checkpoint operation for
  layered COW workdirs, matching the materialized-workdir report contract.
- Automatic update notices use the shared terminal renderer while preserving
  structured-output silence for JSON commands.
- Lane archive and unarchive daemon requests no longer send an unexpected JSON
  body, and interrupted observer retirement with a failed owner can be reopened
  and resumed instead of being reported as a corrupt schema.

## [0.1.1] - 2026-07-29

### Added

- Added `trail upgrade` for installation-aware stable upgrades through
  Homebrew or cargo-dist release installer receipts.
- Added `trail upgrade --check` and non-blocking, once-daily interactive
  update notices. Set `TRAIL_NO_UPDATE_CHECK=1` to disable automatic checks.

### Changed

- **Breaking:** Trail CLI human output now uses the unified outcome-first
  terminal renderer. The old human layouts and `--no-color` option are removed;
  use `--color never` instead.
- **Breaking:** `trail merge-lane` is removed. Use
  `trail lane merge <lane> --into <branch>` for lane-specific merges; the
  `trail merge` command remains for generic branch/ref merges.
- **Breaking:** `POST /v1/branches/{branch}/merge-lane` is removed. Use
  `POST /v1/lanes/{lane}/merge` with the target branch in the required `into`
  JSON field.
- **Breaking:** the generic merge queue is now lane-only. Use
  `trail lane merge-queue`, `/v1/lanes/merges/queue`, and
  `trail.lane_merge_queue_*`; the previous CLI, HTTP, MCP, resource, and
  `merge_queue` storage contracts are removed without aliases. Generic
  branches and refs continue through `trail merge`.
- Added `--format human|plain|json|ndjson`, `--color auto|always|never`, and
  `--pager auto|always|never`. `plain` is deterministic text; JSON and NDJSON
  are the supported contracts for automation.
- Status, diff, history, lane, agent, maintenance, and diagnostic output now
  use responsive tables, ordered checklists, explicit notices, and safe next
  actions. Human output is intentionally not stable for parsing.

## [0.3.0] - 2026-08-10

### Added

- Release 0.3.0.

### Fixed

- Release metadata and version bump housekeeping.

## [0.1.0] - 2026-07-10

### Added

- Local-first operation history, branches, line provenance, and worktree recording.
- Isolated agent lanes with sessions, turns, patches, approvals, gates, and handoffs.
- Conflict-aware lane merges, merge queues, readiness reports, and recovery checkpoints.
- CLI, HTTP daemon, MCP stdio server, ACP relay, and Rust API integration surfaces.
- Backup, restore, filesystem checks, index rebuilding, and maintenance commands.

[Unreleased]: https://github.com/crabbuild/trail/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/crabbuild/trail/compare/v0.3.0...v0.4.0
[0.3.0]: https://github.com/crabbuild/trail/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/crabbuild/trail/compare/v0.1.1...v0.2.0
[0.1.1]: https://github.com/crabbuild/trail/compare/v0.1.0...v0.1.1
[0.1.0]: https://github.com/crabbuild/trail/releases/tag/v0.1.0
