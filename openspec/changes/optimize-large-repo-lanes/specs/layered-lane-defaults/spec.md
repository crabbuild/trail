## ADDED Requirements

### Requirement: Managed lanes default to a qualified layered backend
Trail SHALL resolve an omitted or `auto` workdir mode for managed agent execution to a platform-qualified transparent copy-on-write backend. Trail MUST NOT silently substitute a full copied checkout or a non-layered clone when the desired environment contains mounted components.

#### Scenario: Supported native backend
- **WHEN** an agent creates a lane without an explicit workdir mode on a host with a qualified transparent backend
- **THEN** Trail creates a layered lane using that backend and reports the selected backend and qualification evidence

#### Scenario: No qualified backend
- **WHEN** an agent creates or executes an environment-bearing lane and no qualified transparent backend is available
- **THEN** Trail fails before launching the agent or repository command with platform-specific prerequisite and remediation details

#### Scenario: Explicit non-layered mode
- **WHEN** a user explicitly requests a virtual, sparse, portable-copy, or other non-layered mode
- **THEN** Trail honors that mode for operations that do not require mounted environment outputs and rejects incompatible managed execution explicitly

### Requirement: Lane spawn remains metadata-fast
Trail SHALL create the lane ref, view identity, fresh private uppers, and inherited environment references without eagerly copying the source tree, dependency trees, or generated trees.

#### Scenario: Child of a prepared lane
- **WHEN** a child lane is spawned from a lane with a compatible active environment generation
- **THEN** the child references compatible immutable layers, creates fresh source/generated/scratch uppers, and performs no ecosystem build command during spawn

#### Scenario: Child has mixed compatibility
- **WHEN** only some parent components are compatible with the child source and host
- **THEN** Trail inherits each compatible component independently and records an unresolved reason for every rejected component

### Requirement: Managed execution converges lazily
Before command launch, every managed execution surface SHALL discover the desired component graph, compare it with the active generation, synchronize only stale or unresolved components, reconcile declared runtime resources, and mount the resulting generation.

#### Scenario: Current generation
- **WHEN** the active environment generation exactly satisfies the desired graph
- **THEN** Trail skips synchronization commands and records a `sync` phase with status `skipped` and the matching generation identity

#### Scenario: First execution
- **WHEN** a layered lane has no active generation and discovery finds environment components
- **THEN** Trail prepares and atomically activates the required generation before launching the requested command

#### Scenario: Preparation failure
- **WHEN** discovery, planning, synchronization, runtime reconciliation, or mounting fails
- **THEN** Trail does not launch the requested command and retains structured phase receipts and resumable cleanup state

### Requirement: Mutable lane state is never inherited live
Every child lane MUST receive independent source, generated, scratch, runtime, secret, lease, and synchronization-attempt state even when it inherits immutable layers from its parent.

#### Scenario: Concurrent writes to one seeded path
- **WHEN** parent and child lanes modify the same path above an inherited immutable seed
- **THEN** each lane observes only its own copy-on-write mutation and the immutable lower layer remains unchanged

#### Scenario: Parent runtime remains active
- **WHEN** a child lane is spawned while the parent owns running services or open cache leases
- **THEN** the child inherits neither the runtime allocation nor the lease and reconciles its own resources before execution
