## ADDED Requirements

### Requirement: Lane lifecycle operations are distinct
Trail SHALL expose archive, unarchive, remove, and purge as distinct operations.

#### Scenario: Archive is reversible
- **WHEN** an active lane is archived
- **THEN** its ref, source history, workspace view, private uppers, and environment generation remain available and unarchive restores execution eligibility

#### Scenario: Remove disposes lane-private state
- **WHEN** lane removal completes
- **THEN** the lane has no active generation, environment-view pointer, layer binding, Trail-owned runtime resource, mount owner, generated upper, scratch upper, source upper, or materialized workdir

#### Scenario: Purge erases the compact tombstone
- **WHEN** a removed lane is purged with force and an unambiguous identity
- **THEN** Trail deletes its remaining retirement summary and lane-owned provenance that has no external retention obligation

### Requirement: Removal is crash recoverable
Trail MUST durably record each removal phase before advancing across database, runtime-provider, mount, observer, or filesystem boundaries.

#### Scenario: Process exits during removal
- **WHEN** Trail restarts after a crash at any removal phase
- **THEN** recovery resumes from the last durable phase and repeated recovery converges without adopting foreign resources or deleting an out-of-scope path

#### Scenario: Cleanup cannot complete
- **WHEN** an owned resource cannot be stopped or a private path cannot be deleted
- **THEN** removal returns a structured repair-required result and retains the exact resumable phase and error

### Requirement: Removal makes immutable layers collectable
Trail SHALL remove all view and generation references owned only by a successfully removed lane while preserving layers referenced elsewhere.

#### Scenario: Layer is unique to removed lane
- **WHEN** removal completes and no other active or retained generation references a layer
- **THEN** the next eligible cache GC can reclaim that layer

#### Scenario: Layer is shared with another lane
- **WHEN** removal completes while another lane references the same layer
- **THEN** cache GC preserves the layer and the other lane remains usable

### Requirement: Removed names are reusable
Trail SHALL free a lane's former name only after removal reaches its completed phase.

#### Scenario: Respawn after completed removal
- **WHEN** a user spawns a lane with a name whose previous lane completed removal
- **THEN** Trail creates a new lane identity without conflicting with the historical tombstone

#### Scenario: Respawn during incomplete removal
- **WHEN** a user spawns a lane whose former lane has an incomplete removal
- **THEN** Trail first recovers the removal or returns its structured repair-required result and does not create a second active lane

### Requirement: Removal preserves compact provenance
Trail SHALL retain a bounded removal summary containing lane identity, former name, base/head changes and roots, forced flag, lifecycle timestamps, environment generation IDs, and reclaimed-private-storage accounting.

#### Scenario: Inspect removed lane by identity
- **WHEN** a user addresses a uniquely removed lane by exact lane ID
- **THEN** Trail returns the compact removal provenance without requiring the deleted workspace view or generation graph
