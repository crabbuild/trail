## ADDED Requirements

### Requirement: Extension-contract certification packages
Trail SHALL certify Bazel, Gradle, Maven, and Nix through versioned repository recipes or installed adapter packages governed by the common host lifecycle.

#### Scenario: Expressible system plan
- **WHEN** one of the four systems can declare its exact inputs, host tools, caches, outputs, validation, network/script policy, and portability through the current recipe or plugin protocol
- **THEN** its certification uses that protocol without adding an ecosystem-specific execution bypass in Trail core

#### Scenario: Missing protocol capability
- **WHEN** a required behavior cannot be represented safely by the current protocol
- **THEN** Trail first adds a typed, bounded, denied-by-default protocol capability with SDK and malicious-package conformance coverage

### Requirement: Bazel certification
The Bazel package SHALL model lock/module/workspace configuration as identity, repository/download state as cache-only, and Bazel output roots as lane-private state.

#### Scenario: Bazel A to B to C
- **WHEN** a pinned Bazel repository is built and tested across three semantic successor lanes
- **THEN** all validations pass, shared cache reuse is correctness-neutral, output bases remain isolated, and an identity input change rejects stale state

### Requirement: Gradle certification
The Gradle package SHALL model wrapper verification, dependency locks/version catalogs/settings as identity, Gradle user-home downloads as bounded cache state, and project build/daemon state as lane-private or disposable.

#### Scenario: Gradle A to B to C
- **WHEN** a pinned Gradle repository is built and tested across three semantic successor lanes using the verified wrapper distribution
- **THEN** dependency cache reuse does not share writable project outputs or daemon authority and lock/config changes invalidate exact identity

### Requirement: Maven certification
The Maven package SHALL model wrapper/tool identity, effective lock or reproducible dependency authority, settings without secrets, repository downloads as cache state, and target output as lane-private.

#### Scenario: Maven A to B to C
- **WHEN** a pinned Maven repository is built and tested offline across three semantic successor lanes
- **THEN** local artifact downloads are reused only as cache, target trees remain independent, and POM/lock/tool changes reject stale identity

### Requirement: Nix certification
The Nix package SHALL require flake lock or equivalent pinned evaluation authority and SHALL represent Nix store results as verified external immutable identities rather than writable Trail layers.

#### Scenario: Nix flake A to B to C
- **WHEN** a pinned flake is evaluated, built, and checked across three semantic successor lanes
- **THEN** Trail records exact store-path/content identity and validation evidence, keeps mutable profiles/state lane-private, and rejects unlocked or impure evaluation

### Requirement: Common malicious-package conformance
Every external certification package SHALL pass the adapter contract's hostile-plan, containment, bounds, determinism, recovery, and redaction suite.

#### Scenario: Malicious package proposal
- **WHEN** a package proposes path traversal, mutable host tools, undeclared execution, network/secrets, excessive graph/output size, source shadowing, or nondeterministic plan ordering
- **THEN** the Trail host rejects it before publication and preserves the prior active generation

#### Scenario: Interrupted construction
- **WHEN** the process is killed at any durable staging, validation, publication, or activation boundary
- **THEN** reopen either retains the prior generation or completes the exact committed generation without orphan authority
