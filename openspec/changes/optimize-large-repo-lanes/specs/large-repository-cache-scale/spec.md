## ADDED Requirements

### Requirement: Independent environment components prepare concurrently
Trail SHALL execute ready nodes of the desired component DAG concurrently up to a configured bound while preserving deterministic planning, dependency ordering, result ordering, and atomic generation activation.

#### Scenario: Independent components
- **WHEN** a graph contains multiple stale components with no dependency path between them
- **THEN** Trail may build them concurrently and returns reports in deterministic topological/component order

#### Scenario: Dependent components
- **WHEN** a component has a `build_requires` or `invalidates_with` dependency
- **THEN** Trail does not start that component until every identity-bearing prerequisite has completed successfully

### Requirement: Identical artifact builds are singleflight
At most one live builder per canonical component key SHALL publish a reusable artifact in one cache scope. Concurrent consumers MUST wait with bounded cancellation or attach the completed verified result; they MUST NOT create competing ready layers.

#### Scenario: Two lanes request one missing key
- **WHEN** two lanes concurrently synchronize the same missing component key
- **THEN** one attempt owns the build lease and both successful synchronizations reference the single published layer

#### Scenario: Builder dies
- **WHEN** the lease owner stops before publication
- **THEN** recovery fences the dead owner, preserves any prior ready layer, and permits a new bounded attempt without reusing partial staging bytes

### Requirement: Layer access is lazy and bounded
Trail SHALL serve immutable layers through deterministic manifests and content-addressed projections without eagerly copying or hashing the complete layer for every lane. Metadata caches and prefetch sets MUST be bounded and invalidated by manifest or generation identity.

#### Scenario: Large cold layer
- **WHEN** a lane mounts a layer containing hundreds of thousands of entries but reads only a small subset
- **THEN** lane startup materializes only bounded metadata and accessed or prefetched content rather than copying the entire tree

#### Scenario: Repeated hot command
- **WHEN** a command has prior successful access provenance for the same component and generation identities
- **THEN** Trail may prefetch a bounded hot set and reports prefetched and demand-loaded bytes separately

### Requirement: Cache retention is safe and observable
Trail SHALL enforce configurable maximum bytes, minimum free space, retention age, and per-kind accounting. Active views, retained generations, publication attempts, and live builders MUST pin required layers; garbage collection MUST never treat source uppers or authoritative private state as cache entries.

#### Scenario: Pressure-triggered collection
- **WHEN** cache usage exceeds its configured quota or free space falls below its floor
- **THEN** Trail selects only unpinned recoverable entries in deterministic least-recently-used order and reports planned and reclaimed logical and physical bytes

#### Scenario: Layer pinned by child lane
- **WHEN** a layer is referenced by any active or retained environment generation
- **THEN** cache collection preserves the layer regardless of age

### Requirement: Reuse and rebuild decisions are inspectable
Every sync, execution, promotion, and inheritance report SHALL state the desired key, selected storage identity, decision (`reused`, `built`, `private`, `rejected`, or `failed`), decision source, bytes avoided or written when known, and exact identity edges that caused a rebuild.

#### Scenario: Toolchain-caused rebuild
- **WHEN** source and dependency inputs are unchanged but the resolved compiler identity changes
- **THEN** Trail reports the compiler identity edge as the rebuild reason rather than a generic cache miss

#### Scenario: Cache-hit execution
- **WHEN** a child lane launches with inherited layers and no stale components
- **THEN** the lifecycle report identifies zero environment build commands and the reused layer and generation identities

### Requirement: Large-repository claims require reproducible qualification
Trail MUST qualify correctness and performance with scripted 10k, 100k, and 1M-path fixtures and a real public repository on each claimed backend. Evidence SHALL record host, operating system, backend, cache warmth, lane count, source and generated path counts, logical and physical bytes, spawn-to-exec latency, checkpoint latency, cache hit rate, and skipped gates.

#### Scenario: Twenty independent lanes
- **WHEN** the scale gate creates twenty lanes that inherit one prepared environment and perform independent generated writes
- **THEN** every lane observes its own writes, all shared layers retain their original digest, and the report quantifies storage amplification and startup latency

#### Scenario: Missing native backend evidence
- **WHEN** a platform-specific filesystem gate is skipped or unavailable
- **THEN** Trail labels that platform unverified and does not use the skipped result as passing release evidence
