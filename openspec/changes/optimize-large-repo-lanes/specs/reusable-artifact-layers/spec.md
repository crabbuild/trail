## ADDED Requirements

### Requirement: Every environment output has an explicit storage policy
Trail SHALL classify each declared output using a framework-neutral policy whose contract determines mutability, mount behavior, inheritance, retention, and publication. At minimum the model SHALL distinguish `immutable_shared`, `immutable_seed_private`, `writable_private`, and `disposable`; caches, runtimes, secrets, and externally managed state MUST remain separate typed declarations.

#### Scenario: Consumer-mutated dependency tree
- **WHEN** an adapter declares an output as `immutable_seed_private`
- **THEN** Trail mounts a verified immutable lower layer with a fresh lane-private writable upper

#### Scenario: Lane-private build tree
- **WHEN** an adapter declares an output as `writable_private`
- **THEN** Trail creates or retains that output only in the owning lane's generated upper and does not represent it as a shared layer

#### Scenario: Unsafe policy widening
- **WHEN** repository policy attempts to share output more broadly than its adapter certification permits
- **THEN** Trail rejects the plan or requires an explicit recorded approval allowed by the adapter contract; it never widens sharing silently

### Requirement: Reusable artifacts have complete canonical identities
Trail MUST derive a reusable artifact key from the normalized adapter contract, adapter implementation and distribution identities, declared input content, upstream component keys, command arguments, tool identities, output contract, identity-affecting environment, platform dimensions, portability, and reuse scope. Missing knowledge MUST narrow reuse to an exact source-root identity.

#### Scenario: Unrelated source change outside a certified closure
- **WHEN** a certified adapter declares a complete input closure and an unrelated source path changes
- **THEN** Trail preserves the component key and reuses the existing artifact

#### Scenario: Identity-affecting input changes
- **WHEN** any declared input, toolchain, feature, target, policy, or upstream identity changes
- **THEN** Trail produces a different component key and does not attach the old artifact as current

#### Scenario: Incomplete adapter knowledge
- **WHEN** an adapter cannot prove a complete input closure
- **THEN** Trail includes the complete source-root identity or refuses reusable publication

### Requirement: Private output promotion is explicit and atomic
Trail SHALL provide a promotion operation that snapshots declared private output, validates it, publishes an immutable manifest and layer through staging plus atomic rename, and activates a successor environment generation without modifying the live private upper.

#### Scenario: Manual successful promotion
- **WHEN** a user promotes a declared output after its required validation gates pass
- **THEN** Trail publishes one immutable layer, records its complete provenance, and makes it eligible for compatible child lanes

#### Scenario: Validation failure
- **WHEN** containment, file-type, secret, adapter-semantic, or user-declared validation fails
- **THEN** Trail publishes no ready layer, leaves the active generation unchanged, and reports the failed rule without deleting the lane-private output

#### Scenario: Interrupted publication
- **WHEN** Trail or the host stops during snapshot, validation, manifest creation, or publication
- **THEN** workspace recovery retains the previous active generation and either resumes or removes only staging state owned by that publication attempt

### Requirement: Publication triggers are policy-controlled
Each promotable output SHALL declare one of `manual`, `on_sync`, `successful_gate`, or `never`; an automatic trigger MUST identify the exact execution or gate evidence authorizing publication.

#### Scenario: Successful-gate publication
- **WHEN** an output declares `successful_gate`, its named gate passes against the current source and environment generation, and validation succeeds
- **THEN** Trail may publish the output and records the gate run, source root, command fingerprint, and environment generation in provenance

#### Scenario: Failed command
- **WHEN** the producing command or required gate fails
- **THEN** Trail retains the private output according to lane policy but does not promote it

### Requirement: Artifact inheritance is component-granular and provenance-preserving
Child lanes SHALL inherit each compatible immutable output by layer identity while receiving a fresh writable upper for every seeded output. The spawn and generation reports SHALL include the source generation and a reason for every reused or rejected output.

#### Scenario: Compatible promoted artifact
- **WHEN** Agent B is spawned from Agent A after Agent A published a compatible seed
- **THEN** Agent B references the same immutable layer without running the producer command and writes subsequent changes only to Agent B's upper

#### Scenario: Corrupt inherited layer
- **WHEN** an otherwise compatible layer is missing, corrupt, unsealed, revoked, or outside its allowed scope
- **THEN** Trail rejects that output independently, marks it unresolved, and never falls back to its bytes
