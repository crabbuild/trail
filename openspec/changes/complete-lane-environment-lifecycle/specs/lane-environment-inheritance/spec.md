## ADDED Requirements

### Requirement: Lane forks inherit compatible immutable environment state
When `--from` resolves to a lane, Trail SHALL reuse each compatible immutable environment component and output by reference.

#### Scenario: Compatible parent generation
- **WHEN** a parent lane has a verified active generation whose immutable component keys are compatible with the child source and platform
- **THEN** the child generation references the same layer IDs without duplicating their storage

#### Scenario: Mixed compatibility
- **WHEN** some parent components are compatible and others are not
- **THEN** Trail inherits compatible components and leaves incompatible components unresolved for synchronization with an explicit reason

#### Scenario: No active parent generation
- **WHEN** the source lane has no active environment generation
- **THEN** child spawn succeeds with a fresh unsynchronized environment

### Requirement: Forks always isolate mutable state
Trail MUST create fresh source, generated, and scratch uppers for every child lane and MUST NOT inherit runtime allocations, secret resolutions, leases, sync attempts, or mutable storage identities.

#### Scenario: Parent and child write the same generated path
- **WHEN** both lanes modify a generated path after forking
- **THEN** each lane observes its own private value while both continue reading unchanged immutable layer content by reference

#### Scenario: Parent runtime is active
- **WHEN** a child forks from a parent with running Trail-owned runtime resources
- **THEN** the child does not reference or control the parent's allocation and must reconcile its own runtime resources before execution

### Requirement: Inheritance is provenance preserving
Trail SHALL record the parent generation, child generation, inherited component and layer identities, and rejection reasons.

#### Scenario: Inspect fork generation
- **WHEN** the child environment generation is inspected
- **THEN** its predecessor provenance identifies the parent generation and every inherited or rejected component is explainable
