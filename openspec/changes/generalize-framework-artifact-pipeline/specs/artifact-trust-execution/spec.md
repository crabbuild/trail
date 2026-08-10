## ADDED Requirements

### Requirement: New pipeline phases preserve host authority
Adapters and repository declarations SHALL return bounded proposals, resolver plans, action graphs, validations, and export declarations. Only the Trail host SHALL resolve tools, allocate storage, execute actions, access providers, seal objects, publish envelopes, activate bindings, export source, recover attempts, or collect content.

#### Scenario: Adapter requests direct authority
- **WHEN** a v3 plan requests an absolute host path, database operation, artifact ID assignment, direct mount, provider socket, or publication mutation
- **THEN** Trail rejects it before execution and identifies the responsible package and field

### Requirement: Resolution and construction use separate capability profiles
Resolver actions SHALL receive network access only when explicitly authorized to exact authorities and SHALL record redacted resolution evidence. Constructors and validators SHALL run offline by default against pinned snapshots and managed content; every exception MUST be an identity-bearing, trust-bounded policy input.

#### Scenario: Controlled resolution
- **WHEN** an authorized resolver needs a configured registry
- **THEN** Trail grants only that authority, records its non-secret identity and checksums, and excludes credentials from durable state

#### Scenario: Offline constructor connects
- **WHEN** a constructor declared offline attempts a network connection
- **THEN** the sandbox denies it, construction fails, and no artifact envelope becomes ready

### Requirement: Every phase is deny-by-default and bounded
Trail SHALL enforce normalized bounds for readable inputs, writable candidate/private paths, executable/child graph, environment names, network, runtime, duration, output bytes/entries/depth, captured output, and secret channels. Missing native enforcement MUST fail closed for untrusted resolver, constructor, validator, or export actions.

#### Scenario: Undeclared write
- **WHEN** a repository action writes outside its candidate, isolated home/temp, approved cache, or source-export destination
- **THEN** Trail denies or detects the mutation and publishes no artifact or source change

### Requirement: Secret consumption taints output
Secret bytes MUST NOT enter desired keys, snapshots, content objects, manifests, logs, reports, caches, attestations, source exports, or future remote requests. A producer receiving secret bytes SHALL create only lane-private non-promotable and non-exportable output.

#### Scenario: Signing key used by a producer
- **WHEN** an action receives a signing key through an approved private channel
- **THEN** Trail redacts its evidence and rejects shared sealing or source export of that action's output

### Requirement: Sealing requires host validation and attestation
Before publishing an artifact envelope, Trail SHALL confirm producer termination, unchanged pins, declared-path containment, safe normalized content, bounds, secret policy, complete content identity, required validations/gates, and producer trust. It SHALL publish a deterministic attestation referencing the desired key, tree root, adapter/package/protocol, tools, platform, capability enforcement, policies, and validation receipts.

#### Scenario: Successful command leaves unsafe output
- **WHEN** a producer exits successfully but leaves an escaping link, special file, secret-tainted content, undeclared path, or stale validation input
- **THEN** sealing fails and no ready artifact envelope is published

#### Scenario: Attestation tampering
- **WHEN** stored attestation bytes or references do not match their content identity
- **THEN** attachment fails closed while prior provenance remains inspectable as corrupt evidence

### Requirement: Portable identity does not grant remote trust
Artifact content and attestation identities SHALL avoid machine-specific storage paths and support explicit portability scopes. Matching desired/content identities from another host MUST NOT authorize attachment without a future transport policy validating origin, signatures, manifest integrity, portability, revocation, and local trust.

#### Scenario: Untrusted imported object graph
- **WHEN** matching objects arrive outside the local trust scope
- **THEN** Trail rejects them or retains them as untrusted candidates and does not bind them to a generation
