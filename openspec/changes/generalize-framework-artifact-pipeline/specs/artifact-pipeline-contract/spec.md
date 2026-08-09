## ADDED Requirements

### Requirement: Discovery reports recognized incomplete components
Trail SHALL return a typed proposal from side-effect-free source markers even when lock state, tools, platform capabilities, or policy are incomplete. Each proposal SHALL be `ready`, `resolvable`, `blocked`, `unsupported`, or `ambiguous` and SHALL contain stable reasons and recovery actions.

#### Scenario: Manifest without lock state
- **WHEN** a recognized dependency manifest exists without a source or Trail-managed resolution snapshot
- **THEN** discovery reports the component as `resolvable` or `blocked` and does not silently omit it

#### Scenario: Discovery remains read-only
- **WHEN** Trail discovers built-in, plugin, or repository-defined components
- **THEN** Trail invokes no package manager, compiler, repository action, network endpoint, runtime provider, or secret provider

### Requirement: Resolution snapshots are explicit environment metadata
Trail SHALL support an optional resolver phase that converts a pinned proposal into an immutable content-addressed snapshot. Snapshot bytes and provenance SHALL remain environment metadata and MUST NOT become source unless a user authorizes a normal source export/materialization operation.

#### Scenario: Explicit resolution
- **WHEN** a resolvable proposal lacks a snapshot and the user runs its reported resolve operation
- **THEN** Trail executes the bounded authorized resolver, validates one snapshot, stores it outside source history, and replans against its identity

#### Scenario: Refresh is deliberate
- **WHEN** a valid snapshot exists and neither its identity inputs nor an explicit `--refresh` request changes
- **THEN** Trail reuses it and does not select newer dependencies merely because time passed

### Requirement: Resolution and construction extend existing convergence
Trail SHALL feed proposals, snapshots, desired keys, validated artifacts, and private outputs into the existing component DAG, environment generation, promotion, COW binding, managed execution, retirement, and collection operations. It MUST NOT introduce a parallel generation or mount authority.

#### Scenario: First resolved execution
- **WHEN** every missing snapshot is available and the active generation is incomplete
- **THEN** existing environment synchronization constructs or reuses required nodes and atomically activates one complete successor generation before command launch

#### Scenario: Phase failure
- **WHEN** resolution, construction, validation, sealing, or activation fails
- **THEN** the previous active generation remains authoritative and the report identifies the exact failed phase and safe recovery action

### Requirement: Repository pipelines extend trail.environment.toml
Trail SHALL add the common pipeline through an explicitly versioned extension of `trail.environment.toml` and `.trail/environment.toml`. Version-1 documents SHALL retain their existing interpretation; version-2 documents SHALL compile resolution, typed actions, validations, outputs, capabilities, and exports to the same normalized host graph as built-in and plugin adapters.

#### Scenario: Version-2 custom framework
- **WHEN** a repository declares a bounded version-2 resolver, constructor, validation, artifact output, and source export
- **THEN** Trail validates and plans them without framework-specific lifecycle code or shell interpolation

#### Scenario: Version-1 compatibility
- **WHEN** Trail reads an existing `trail.environment/v1` document
- **THEN** it preserves the shipped v1 defaults and restrictions and does not infer v2 resolution or export authority

### Requirement: Existing artifact policies remain the storage vocabulary
Pipeline artifact outputs SHALL use the shipped `immutable_shared`, `immutable_seed_private`, `writable_private`, or `disposable` policy. Caches, runtimes, secrets, and external resources SHALL remain separately typed, and generated source SHALL use a source-export declaration rather than a fifth artifact policy.

#### Scenario: Seeded mutable output
- **WHEN** an output is `immutable_seed_private`
- **THEN** Trail attaches its verified immutable artifact through the existing lower-layer binding and routes all mutations to fresh lane-private state

#### Scenario: Generated source
- **WHEN** a component generates bytes intended for Git review
- **THEN** those bytes remain artifact/private output until an authorized source export writes them through normal source guardrails

### Requirement: Source export is explicit and reviewable
Trail SHALL export generated source only from a declared validated candidate or artifact subtree into a normalized repository-relative destination. Export MUST pin source, desired, content, validation, and destination state and SHALL apply ignore, secret, collision, guardrail, diff, checkpoint, and Git-handoff semantics.

#### Scenario: Successful source export
- **WHEN** a user authorizes a current declared export and the destination passes all source checks
- **THEN** Trail writes the result into the lane source upper and exposes it in normal lane diff and review

#### Scenario: Stale or conflicting export
- **WHEN** the source root, artifact identity, validation receipt, destination, or existing source content changes after export planning
- **THEN** Trail writes nothing and reports the stale or conflicting evidence
