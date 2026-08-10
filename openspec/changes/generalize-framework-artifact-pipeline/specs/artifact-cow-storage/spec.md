## ADDED Requirements

### Requirement: Pure content manifests are deterministic
Trail SHALL represent sealed artifact content as versioned deterministic directory, file, blob, chunk-list, and chunk objects. Pure content identity MUST exclude desired keys, layer IDs, generation IDs, attempt IDs, storage paths, timestamps, and producer provenance.

#### Scenario: Equivalent trees
- **WHEN** two producers seal equivalent normalized trees in different traversal orders
- **THEN** they produce the same tree-content root

#### Scenario: Unsafe entry
- **WHEN** a candidate contains traversal, non-normalized names, a case collision, escaping link, prohibited metadata, or unsupported file type
- **THEN** Trail rejects sealing before any ready artifact envelope is reachable

### Requirement: File content is deduplicated independently of artifacts
Trail SHALL store small files as immutable whole-blob objects and large files through a versioned content-defined chunk profile plus complete-file digest. Artifact envelopes and successor manifests SHALL reference existing unchanged objects instead of copying their bytes.

#### Scenario: Different keys share files
- **WHEN** artifacts with different desired keys contain equal files or chunks
- **THEN** Trail stores one authoritative content object for each equal identity and both manifests reference it

#### Scenario: Small successor change
- **WHEN** a successor modifies a bounded subset of files or chunks
- **THEN** Trail creates only affected directory/file/chunk objects plus metadata and reuses all unchanged object identities

### Requirement: CAS objects are authoritative and materializations are rebuildable
Ready artifacts SHALL remain reconstructible from durable manifests and content objects. Layer directories, backend projections, prefetch data, and verified materialization caches MUST NOT be the only copy of artifact bytes and SHALL be independently reclaimable.

#### Scenario: Materialization is removed
- **WHEN** GC removes an unmounted artifact materialization while its envelope remains retained
- **THEN** a later attachment reconstructs or lazily serves the same verified content root without rebuilding the framework component

#### Scenario: Missing content object
- **WHEN** an envelope references a missing or corrupt content object
- **THEN** Trail marks the artifact corrupt, refuses new attachment atomically, and reports repair or rebuild guidance without serving partial surviving bytes

### Requirement: CAS sealing integrates with existing promotion and COW bindings
Trail SHALL publish content objects and a ready artifact envelope before the existing successor-generation activation transaction. Existing immutable lower, seeded private upper, whiteout, promotion-quiescence, fork-inheritance, and rollback behavior SHALL remain authoritative.

#### Scenario: Promoted private output
- **WHEN** a declared private output is quiesced, validated, and promoted
- **THEN** Trail seals its content into CAS, activates a successor generation through the existing publication fence, and leaves the live private upper unchanged

#### Scenario: Sibling mutation
- **WHEN** sibling lanes attach one CAS-backed immutable seed and mutate or delete the same lower path
- **THEN** each sees only its private mutation/whiteout and full verification preserves the common content root

### Requirement: Publication and recovery preserve exact ownership
Resolution, construction, validation, object sealing, envelope publication, generation activation, materialization, retirement, and collection SHALL use durable fenced state and confined idempotent operations. Recovery MUST delete or resume only state owned by the exact attempt.

#### Scenario: Stop after envelope publication
- **WHEN** Trail stops after a ready envelope is durable but before generation activation
- **THEN** recovery either completes the exact fenced activation or leaves an unbound collectable artifact while preserving the prior generation

### Requirement: Reachability and accounting operate over shared objects
Trail SHALL retain every object reachable from active or retained generations, live/recoverable attempts, leases, quarantines, backups, and explicit holds. Accounting SHALL distinguish logical artifact bytes, unique authoritative bytes, bytes shared with other artifacts, materialization bytes, lane-private bytes, and reclaimable bytes without double counting.

#### Scenario: Last artifact reference expires
- **WHEN** the final envelope and hold referencing a content object expire
- **THEN** deterministic GC can reclaim that object while preserving every object reachable from another artifact

#### Scenario: Twenty lanes share one tree
- **WHEN** twenty lanes attach one artifact and make small private changes
- **THEN** accounting reports one authoritative shared tree, any reclaimable materializations, and twenty lane-private deltas rather than twenty authoritative copies
