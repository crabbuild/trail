## ADDED Requirements

### Requirement: Go multi-module workspace graph
Trail SHALL discover a contained `go.work` graph as one environment component, include every workspace member and graph authority in identity, and construct its vendor output without mutating source.

#### Scenario: Contained workspace handoff
- **WHEN** agents A, B, and C successively edit modules in one pinned `go.work` repository
- **THEN** each child starts from its parent's semantic checkpoint, receives a distinct exact component identity where source-sensitive vendor inputs change, and may seed construction only from a compatible predecessor

#### Scenario: Escaping workspace member
- **WHEN** `go.work` contains an absolute, parent-traversing, symlink-escaping, duplicate, or over-limit member
- **THEN** discovery or planning fails closed before invoking Go or publishing an environment generation

### Requirement: Yarn and Bun frozen dependency gates
Trail SHALL apply manager-specific frozen resolution and installation contracts for Yarn Classic and Bun and SHALL certify each against a pinned real repository.

#### Scenario: Source-only successor
- **WHEN** a source-only A → B → C edit leaves the manager lock and policy unchanged
- **THEN** Yarn or Bun retains the same dependency key and immutable seed while every lane receives an independent writable upper

#### Scenario: Dependency invalidation
- **WHEN** the lockfile, package-manager identity, lifecycle policy, platform-sensitive contract, or dependency manifest changes
- **THEN** Trail rejects stale output and resolves a new exact component rather than claiming reuse

#### Scenario: Yarn PnP repository
- **WHEN** a Yarn repository selects Plug'n'Play instead of a `node_modules` linker
- **THEN** the built-in Node adapter reports an explicit unsupported PnP contract and publishes no empty or misleading layer

### Requirement: Frozen uv project synchronization
Trail SHALL install `uv.lock` projects with `uv sync --frozen` semantics into a lane-private virtual environment and SHALL use the shared uv cache only as performance state.

#### Scenario: Locked project installation
- **WHEN** a contained project has a valid `pyproject.toml` and `uv.lock`
- **THEN** synchronization creates a complete project environment, validates locked dependencies/project installation, and activates direct private `.venv` bindings without source checkpoint pollution

#### Scenario: Frozen lock mismatch
- **WHEN** project metadata and `uv.lock` are inconsistent or uv would need to update the lock
- **THEN** synchronization fails without changing the repository lockfile or active environment generation

#### Scenario: Python workspace escape
- **WHEN** uv workspace/source metadata resolves outside the pinned repository or exceeds graph bounds
- **THEN** Trail rejects the plan before installation

### Requirement: Modern CMake private build environment
Trail SHALL model CMake presets, Ninja, contained toolchain files, ccache, and pinned vcpkg manifest mode while retaining lane-private mutable build trees.

#### Scenario: Preset and Ninja build
- **WHEN** a repository selects a contained configure preset using Ninja
- **THEN** the selected expanded preset, generator, CMake/Ninja/compiler/toolchain identities, and policy determine the component identity and managed commands use the lane-private build directory

#### Scenario: Compiler cache reuse
- **WHEN** compatible lanes use ccache with identical correctness identity
- **THEN** they share only the adapter-managed compiler cache namespace and never share a writable CMake build tree

#### Scenario: Pinned vcpkg manifest
- **WHEN** the repository has a vcpkg manifest with a pinned baseline and a verified host vcpkg/toolchain identity
- **THEN** Trail records dependency-manager identity, confines caches and installed/build output by policy, and can reproduce construction offline from prewarmed inputs

#### Scenario: Unsafe preset or toolchain include
- **WHEN** a preset include, toolchain file, vcpkg path, or generated directory escapes the component root or declared host toolchain boundary
- **THEN** Trail fails closed before configure

### Requirement: Approved Node lifecycle and native addons
Trail SHALL disable Node lifecycle scripts by default and SHALL execute them only under an exact, versioned, committed approval and native sandbox policy.

#### Scenario: Approved native addon
- **WHEN** a lock-pinned package and lifecycle phase exactly match the committed approval and all compiler/platform identities are available
- **THEN** Trail may build the addon with denied network and declared writable outputs, and scopes the result to the exact ABI, platform, toolchain, manager, lock, and approval identity

#### Scenario: Unapproved transitive script
- **WHEN** installation attempts any lifecycle script or native build not selected by the approval
- **THEN** construction terminates, publishes no shared result, and records a bounded redacted policy failure

#### Scenario: Undeclared side effect
- **WHEN** an approved script attempts network access or writes outside declared outputs/caches/temp
- **THEN** the host sandbox denies the operation and the active generation remains unchanged

### Requirement: Semantic ecosystem certification
Trail SHALL label an ecosystem variant certified only when canonical local and hosted evidence proves its adapter contract, real-tool behavior, and semantic A → B → C handoff.

#### Scenario: Certification evidence accepted
- **WHEN** evidence names pinned repository/tool/adapter identities, three exact lane roots and ancestry, expected reuse/private outputs, validations, invalidation cases, and raw evidence hashes
- **THEN** the verifier accepts it deterministically and documentation may mark that variant certified for the tested platforms

#### Scenario: Incomplete or contradictory evidence
- **WHEN** evidence omits a required assertion, claims reuse with changed correctness identity, reuses private output across lanes, or has mismatched raw hashes
- **THEN** the verifier rejects it and CI cannot promote the certification
