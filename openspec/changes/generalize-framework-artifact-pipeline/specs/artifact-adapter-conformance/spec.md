## ADDED Requirements

### Requirement: Protocol v3 expresses the incremental common pipeline
Trail SHALL negotiate a bounded `trail.environment-adapter/v3` protocol for incomplete proposals, resolution plans/snapshots, typed action phases, validations, desired/content/artifact identities, capability profiles, source exports, attestations, and quarantine evidence. V1/v2 packages SHALL retain their exact existing meanings and MUST NOT obtain v3-only authority through missing-field defaults.

#### Scenario: Multi-version package
- **WHEN** host and package support v3
- **THEN** they select v3 explicitly and include selected protocol/package identity in planning identity

#### Scenario: V2 compatibility
- **WHEN** a package supports only v2
- **THEN** Trail uses its deterministic v2 conversion and rejects requests for resolution, source export, or v3 certification

### Requirement: SDK and repository schema reject invalid plans early
The SDK v3 builders and `trail.environment/v2` parser SHALL enforce canonical ordering, bounded collections, normalized paths, fixed argv, graph validity, phase/output compatibility, capability ceilings, and source-export constraints. The host MUST repeat validation for every untrusted response.

#### Scenario: Invalid authoring combination
- **WHEN** a declaration combines secret-tainted construction with shared publication or source export
- **THEN** authoring validation rejects it before serialization or execution

### Requirement: Certification is behavior-based
Trail SHALL certify built-in, plugin, and repository-defined producers against the same applicable discovery, resolution, identity, validation, sealing, COW, recovery, invalidation, export, retirement, and collection fixtures. Framework names alone MUST NOT grant reuse, portability, trust, or sandbox capabilities.

#### Scenario: Custom framework passes the contract
- **WHEN** a repository-defined pipeline satisfies the applicable fixtures under repository trust
- **THEN** it receives only the reuse and execution behavior permitted by that trust tier without core framework-specific lifecycle code

### Requirement: Representative artifact shapes receive real-tool evidence
Qualification SHALL cover at least a dependency-tree resolver/installer, compiled incremental tree, bundler/framework build, path-bound private environment, metadata-only external artifact, and repository-defined custom pipeline. Every gate SHALL prove actual tool execution, identities, reuse/invalidation, lane isolation, source-diff classification, disposal, and storage accounting.

#### Scenario: Framework composition
- **WHEN** a Vite- or Next.js-like build depends on a Node-like resolution/dependency component
- **THEN** the report shows distinct graph nodes and rebuilds application output without reinstalling unchanged dependencies

### Requirement: Native evidence covers CAS-backed COW
Every claimed NFS, FUSE, or Dokan backend SHALL prove CAS-backed immutable lower integrity, private writes/whiteouts, fork inheritance, materialization recovery, promotion, source export, retirement, and 1/5/20-lane storage behavior on the owning platform. A skipped gate MUST remain unverified.

#### Scenario: Sibling isolation under native backend
- **WHEN** sibling lanes share one CAS-backed seed and make conflicting mutations
- **THEN** each lane sees its own result, the lower content root stays valid, and evidence quantifies shared/materialized/private bytes

### Requirement: Public interfaces share the extended typed reports
Rust, CLI JSON/NDJSON, HTTP/OpenAPI, and MCP SHALL project the same resolution, identity, verification, quarantine, attestation, export, recovery, and accounting models. Human rendering SHALL provide a readable summary while structured output remains the automation contract.

#### Scenario: Resolve report parity
- **WHEN** equivalent resolution is invoked through supported interfaces
- **THEN** proposal, snapshot, attempt, authority, identity, decision, and recovery fields have the same semantics and deterministic ordering

### Requirement: Protocol and artifact inputs are bounded and adversarially tested
Trail SHALL bound frames, graphs, patterns, expanded inputs, actions, output entries/bytes/depth, snapshots, objects, chunks, validation output, subprocesses, concurrency, pagination, and diagnostic retention. Limit exhaustion SHALL return explicit failure or truncation and MUST NOT become an empty successful result.

#### Scenario: Oversized artifact manifest
- **WHEN** a producer exceeds declared entry, byte, depth, path, or chunk limits
- **THEN** Trail aborts sealing, retains no ready envelope, and reports the exact exceeded limit
